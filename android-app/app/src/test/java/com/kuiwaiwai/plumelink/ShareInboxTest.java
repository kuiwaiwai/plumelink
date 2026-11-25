package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;
import static org.junit.Assert.*;

public class ShareInboxTest {
    private static List<String> files(String... names) {
        ArrayList<String> result = new ArrayList<>();
        for (String name : names) result.add("content://share.example/" + name);
        return result;
    }
    @Test public void confirmationIsRequiredAndOnlyAReadyCurrentSelectionCanBeConsumed() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        String preparing = inbox.revision();
        assertFalse(inbox.consume(preparing));
        inbox.metadata(inbox.files().get(0), "first.txt", 0, 0);
        assertFalse(inbox.consume(preparing));
        String ready = inbox.revision();
        assertTrue(inbox.ready()); assertTrue(inbox.consume(ready));
        assertFalse(inbox.consume(ready)); assertTrue(inbox.empty());
    }
    @Test public void aNewShareDoesNotGetConsumedByAnOldConfirmation() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        inbox.metadata(inbox.files().get(0), "first.txt", 16, 0);
        String old = inbox.revision(); List<ShareInbox.File> inFlight = inbox.files();
        inbox.add(files("second"));
        assertFalse(inbox.consume(old));
        assertEquals(2, inbox.files().size()); assertEquals(1, inFlight.size());
        assertThrows(UnsupportedOperationException.class, inFlight::clear);
    }
    @Test public void exceedingTheLimitPreservesThePreviousSelectionAndRevision() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        String previous = inbox.revision();
        List<String> tooMany = new ArrayList<>();
        for (int i = 0; i < 16; i++) tooMany.add("content://share.example/new-" + i);
        assertFalse(inbox.add(tooMany));
        assertEquals(previous, inbox.revision()); assertEquals(1, inbox.files().size());
        assertFalse(inbox.add(Arrays.asList("content://share.example/valid", null)));
        assertEquals(previous, inbox.revision());
    }
    @Test public void repeatedUrisKeepTheirIdentityAndDoNotResetMetadata() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first", "first"));
        ShareInbox.File file = inbox.files().get(0);
        inbox.metadata(file, "first.txt", 8, 0);
        String revision = inbox.revision();
        inbox.add(files("first"));
        assertEquals(revision, inbox.revision()); assertTrue(inbox.ready());
        assertEquals(file.id(), inbox.files().get(0).id());
    }
    @Test public void aRemovedFileCannotReturnThroughALateMetadataResult() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        ShareInbox.File old = inbox.files().get(0);
        inbox.clear(inbox.revision()); inbox.add(files("first"));
        inbox.metadata(old, "wrong.txt", 8, 0);
        assertTrue(inbox.files().get(0).pending());
        assertNotEquals(old.id(), inbox.files().get(0).id());
    }
    @Test public void staleRemoveAndClearDoNotAlterANewerShare() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        String previous = inbox.revision(); String id = inbox.files().get(0).id();
        inbox.add(files("second"));
        inbox.remove(previous, id); inbox.clear(previous);
        assertEquals(2, inbox.files().size());
    }
    @Test public void unreadableFilesMustBeRemovedBeforeSendingTheRemainingSelection() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first", "broken"));
        inbox.metadata(inbox.files().get(0), "first.txt", 8, 0);
        inbox.metadata(inbox.files().get(1), "broken.txt", -1, 1);
        assertFalse(inbox.ready());
        inbox.remove(inbox.revision(), inbox.files().get(1).id());
        assertTrue(inbox.ready());
    }
    @Test public void retryCreatesANewMetadataAttemptAndRejectsItsOldResultAndConfirmation() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first"));
        ShareInbox.File failed = inbox.files().get(0);
        inbox.metadata(failed, "first.txt", 8, 1);
        String previous = inbox.revision();
        ShareInbox.File retried = inbox.retry(previous, failed.id());
        assertNotNull(retried); assertTrue(retried.pending()); assertEquals(0, retried.error());
        assertEquals(failed.uri(), retried.uri()); assertEquals("first.txt", retried.name());
        assertNotEquals(failed.id(), retried.id()); assertNotEquals(previous, inbox.revision());
        assertFalse(inbox.ready()); assertFalse(inbox.consume(previous));
        inbox.metadata(failed, "late-wrong.txt", 16, 0);
        assertTrue(inbox.files().get(0).pending()); assertEquals("first.txt", inbox.files().get(0).name());
        inbox.metadata(retried, "current.txt", 16, 0);
        assertTrue(inbox.ready()); assertEquals("current.txt", inbox.files().get(0).name());
    }
    @Test public void retryNeverResetsSuccessfulOrPendingFilesOrAcceptsAStaleSelection() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("ready", "pending", "failed"));
        ShareInbox.File ready = inbox.files().get(0), pending = inbox.files().get(1), failed = inbox.files().get(2);
        inbox.metadata(ready, "ready.txt", 8, 0); inbox.metadata(failed, "failed.txt", -1, 1);
        String current = inbox.revision(); List<ShareInbox.File> previous = inbox.files();
        assertNull(inbox.retry("old", failed.id())); assertNull(inbox.retry(current, "missing"));
        assertNull(inbox.retry(current, ready.id())); assertNull(inbox.retry(current, pending.id()));
        assertEquals(current, inbox.revision()); assertEquals(previous, inbox.files());
        ShareInbox.File retried = inbox.retry(current, failed.id()); assertNotNull(retried);
        assertNull(inbox.retry(current, failed.id()));
        assertEquals(ready.id(), inbox.files().get(0).id()); assertEquals(pending.id(), inbox.files().get(1).id());
    }
    @Test public void retryIsPerFileAndNeverConsumesOrRestartsTheOtherFailures() {
        ShareInbox inbox = new ShareInbox(); inbox.add(files("first", "second"));
        ShareInbox.File first = inbox.files().get(0), second = inbox.files().get(1);
        inbox.metadata(first, "first.txt", 8, 1); inbox.metadata(second, "second.txt", 8, 2);
        ShareInbox.File retry = inbox.retry(inbox.revision(), first.id());
        assertNotNull(retry); assertTrue(retry.pending()); assertEquals(2, inbox.files().size());
        assertEquals(second.id(), inbox.files().get(1).id()); assertEquals(2, inbox.files().get(1).error());
        assertFalse(inbox.ready());
        inbox.clear(inbox.revision()); inbox.metadata(retry, "late.txt", 8, 0);
        assertTrue(inbox.empty());
    }
}
