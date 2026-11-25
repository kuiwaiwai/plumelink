use serde::Serialize;
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointVolume {
    pub id: String,
    pub volume_percent: Option<u8>,
    pub muted: Option<bool>,
    pub error: String,
}

impl EndpointVolume {
    pub(super) fn unavailable(id: &str, message: &str) -> Self {
        Self {
            id: id.into(),
            error: message.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointVolumes {
    pub output: Option<EndpointVolume>,
    pub recording: Option<EndpointVolume>,
}

impl EndpointVolumes {
    fn unavailable(output: &str, recording: &str, message: &str) -> Self {
        Self {
            output: (!output.is_empty()).then(|| EndpointVolume::unavailable(output, message)),
            recording: (!recording.is_empty())
                .then(|| EndpointVolume::unavailable(recording, message)),
        }
    }
}

struct Reader {
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl Reader {
    async fn query<F>(&self, output: &str, recording: &str, probe: F) -> EndpointVolumes
    where
        F: FnOnce() -> EndpointVolumes + Send + 'static,
    {
        if output.is_empty() && recording.is_empty() {
            return EndpointVolumes::default();
        }
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            return EndpointVolumes::unavailable(output, recording, "音量检查进行中，请稍后刷新");
        };
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            probe()
        });
        match tokio::time::timeout(self.timeout, task).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => EndpointVolumes::unavailable(
                output,
                recording,
                "无法读取设备音量，请刷新设备状态。",
            ),
            Err(_) => {
                EndpointVolumes::unavailable(output, recording, "读取设备音量超时，请稍后刷新。")
            }
        }
    }
}

pub async fn read(output: String, recording: String) -> EndpointVolumes {
    static READER: OnceLock<Reader> = OnceLock::new();
    let reader = READER.get_or_init(|| Reader {
        permits: Arc::new(Semaphore::new(2)),
        timeout: Duration::from_millis(1500),
    });
    let native_output = output.clone();
    let native_recording = recording.clone();
    reader
        .query(&output, &recording, move || {
            super::wasapi::endpoint_volumes(&native_output, &native_recording)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn timed_out_native_reads_keep_their_slot_until_the_driver_returns() {
        let reader = Arc::new(Reader {
            permits: Arc::new(Semaphore::new(1)),
            timeout: Duration::from_millis(25),
        });
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let first = reader.clone();
        let task = tokio::spawn(async move {
            first
                .query("out", "in", move || {
                    let _ = entered.send(());
                    let _ = waiting.recv_timeout(Duration::from_secs(3));
                    EndpointVolumes::default()
                })
                .await
        });
        started.await.unwrap();
        let timeout = task.await.unwrap();
        assert!(timeout.output.unwrap().volume_percent.is_none());
        assert!(timeout.recording.unwrap().muted.is_none());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let busy = reader
            .query("next", "", move || {
                count.fetch_add(1, Ordering::Relaxed);
                EndpointVolumes::default()
            })
            .await;
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(busy.output.unwrap().error.contains("进行中"));
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while reader.permits.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let recovered = reader
            .query("next", "", || EndpointVolumes {
                output: Some(EndpointVolume {
                    id: "next".into(),
                    volume_percent: Some(70),
                    muted: Some(false),
                    error: String::new(),
                }),
                recording: None,
            })
            .await;
        assert_eq!(recovered.output.unwrap().volume_percent, Some(70));
    }

    #[tokio::test]
    async fn an_unconfigured_route_does_not_open_any_audio_device() {
        let reader = Reader {
            permits: Arc::new(Semaphore::new(1)),
            timeout: Duration::from_secs(1),
        };
        let value = reader
            .query("", "", || {
                panic!("unconfigured devices must not be queried")
            })
            .await;
        assert!(value.output.is_none() && value.recording.is_none());
    }

    #[tokio::test]
    async fn worker_failure_reports_unknown_levels_and_releases_its_slot() {
        let reader = Reader {
            permits: Arc::new(Semaphore::new(1)),
            timeout: Duration::from_secs(1),
        };
        let value = reader
            .query("out", "", || panic!("simulated native query failure"))
            .await;
        let output = value.output.unwrap();
        assert_eq!(output.id, "out");
        assert!(output.volume_percent.is_none() && output.muted.is_none());
        assert!(!output.error.is_empty());
        assert!(value.recording.is_none());
        assert_eq!(reader.permits.available_permits(), 1);
    }
}
