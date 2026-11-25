package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class NativeAuthorizationFailureTest {
    @Test public void fileAuthorizationFailuresHaveDistinctRecoveryInstructions() {
        for (int status : new int[]{401, 403}) {
            int reason = status == 401 ? R.string.ui_transfer_pairing_expired : R.string.ui_transfer_permission_denied;
            assertEquals(reason, ResumableDownload.rejectionReason(status));
            assertEquals(reason, ResumableUpload.rejectionReason(status));
        }
        assertEquals(R.string.ui_transfer_download_rejected, ResumableDownload.rejectionReason(404));
        assertEquals(R.string.ui_transfer_upload_rejected, ResumableUpload.rejectionReason(413));
    }
    @Test public void audioRevocationAndExpiredCredentialsDoNotLookLikeAnOrdinaryStop() {
        assertEquals("麦克风权限已关闭，请在电脑端的设备权限中开启麦克风，然后重新开始收音", MicrophoneCapture.closingMessage(4403));
        assertEquals("配对凭据已失效，请重新连接电脑并配对后再开启麦克风", MicrophoneCapture.closingMessage(4003));
        assertEquals("电脑已停止接收麦克风", MicrophoneCapture.closingMessage(4000));
    }
}
