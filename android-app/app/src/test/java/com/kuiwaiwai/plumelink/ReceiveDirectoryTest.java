package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class ReceiveDirectoryTest {
    @Test public void storageRootsUseReadableLocalizedNames() {
        String authority = "com.android.externalstorage.documents";
        assertEquals("Internal storage", ReceiveDirectory.labelPath(authority,"primary:","Internal storage","External storage"));
        assertEquals("内部存储空间", ReceiveDirectory.labelPath(authority,"primary:","内部存储空间","外部存储空间"));
        assertEquals("外部儲存空間", ReceiveDirectory.labelPath(authority,"ABCD-1234:","內部儲存空間","外部儲存空間"));
    }

    @Test public void folderPathsAndOtherProvidersKeepTheirOriginalNames() {
        String path = "Documents/接收文件/Long folder name/report:archive";
        assertEquals(path, ReceiveDirectory.labelPath("com.android.externalstorage.documents","primary:"+path,"Internal storage","External storage"));
        assertEquals("primary:", ReceiveDirectory.labelPath("example.provider","primary:","Internal storage","External storage"));
        assertEquals("Downloads", ReceiveDirectory.labelPath("com.android.externalstorage.documents","Downloads","Internal storage","External storage"));
    }
}
