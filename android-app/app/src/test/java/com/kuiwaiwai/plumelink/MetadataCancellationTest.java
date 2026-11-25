package com.kuiwaiwai.plumelink;

import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;
import static org.junit.Assert.*;

public class MetadataCancellationTest {
    @Test public void localCancellationRejectsTheReadWithoutAProviderOrRemoteSignal() {
        TransferFailure failure = assertThrows(TransferFailure.class,
            () -> ResumableUpload.readMetadata(null, null, null, () -> true));
        assertEquals(R.string.ui_transfer_canceled, failure.reason());
    }
    @Test public void cancellationAfterAQueryPreventsFallbackEvenWhenRemoteCancelCannotRun() throws Exception {
        AtomicBoolean cancelled = new AtomicBoolean();
        ResumableUpload.checkMetadataCancellation(null, cancelled::get);
        cancelled.set(true);
        TransferFailure failure = assertThrows(TransferFailure.class,
            () -> ResumableUpload.checkMetadataCancellation(null, cancelled::get));
        assertEquals(R.string.ui_transfer_canceled, failure.reason());
        AtomicBoolean nextRead = new AtomicBoolean();
        ResumableUpload.checkMetadataCancellation(null, nextRead::get);
        assertTrue(cancelled.get());
    }
}
