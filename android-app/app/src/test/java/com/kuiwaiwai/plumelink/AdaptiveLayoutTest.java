package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static com.kuiwaiwai.plumelink.AdaptiveLayout.Workspace.*;

public class AdaptiveLayoutTest {
    @Test public void workspaceUsesWidthHeightAndAvailableAspect() {
        assertEquals(STACKED, AdaptiveLayout.workspace(599, 480));
        assertEquals(NAVIGATION_RAIL, AdaptiveLayout.workspace(600, 480));
        assertEquals(NAVIGATION_RAIL, AdaptiveLayout.workspace(839, 480));
        assertEquals(NAVIGATION_RAIL, AdaptiveLayout.workspace(840, 479));
        assertEquals(TWO_PANE, AdaptiveLayout.workspace(840, 480));
        assertEquals(STACKED, AdaptiveLayout.workspace(840, 900));
        assertEquals(STACKED, AdaptiveLayout.workspace(600, 600));
    }

    @Test public void gridUsesTheLargestColumnCountThatFitsTheMeasuredCells() {
        assertEquals(3, AdaptiveLayout.fittingColumns(316, 8, 100, 3));
        assertEquals(2, AdaptiveLayout.fittingColumns(315, 8, 100, 3));
        assertEquals(2, AdaptiveLayout.fittingColumns(208, 8, 100, 3));
        assertEquals(1, AdaptiveLayout.fittingColumns(207, 8, 100, 3));
        assertEquals(1, AdaptiveLayout.fittingColumns(80, 8, 100, 3));
        assertEquals(2, AdaptiveLayout.fittingColumns(316, 8, 129, 3));
    }

    @Test public void rowsSwitchAtTheirMeasuredContentWidth() {
        assertEquals(true, AdaptiveLayout.fitsInline(220, 111.5f, 100.5f, 8));
        assertEquals(false, AdaptiveLayout.fitsInline(219, 111.5f, 100.5f, 8));
        assertEquals(false, AdaptiveLayout.fitsInline(220, 111.5f, 130.5f, 8));
    }
    @Test public void dialSizesStayDistinctOnShortLandscapeScreens() {
        for (int requested : new int[]{176, 208, 256})
            assertEquals(requested, AdaptiveLayout.dialDiameterDp(320, requested));
        int small = AdaptiveLayout.dialDiameterDp(182, 176);
        int standard = AdaptiveLayout.dialDiameterDp(182, 208);
        int large = AdaptiveLayout.dialDiameterDp(182, 256);
        assertTrue(small >= 152 && small < standard && standard < large && large <= 182);
    }
    @Test public void connectionActionsKeepTheComputerListUsableAtCompactHeights() {
        assertEquals(AdaptiveLayout.Connection.ACTION_RAIL,AdaptiveLayout.connection(640,336));
        assertEquals(AdaptiveLayout.Connection.ACTION_RAIL,AdaptiveLayout.connection(840,480));
        assertEquals(AdaptiveLayout.Connection.STACKED,AdaptiveLayout.connection(360,616));
        assertEquals(AdaptiveLayout.Connection.SCROLLING,AdaptiveLayout.connection(360,260));
        assertEquals(AdaptiveLayout.Connection.SCROLLING,AdaptiveLayout.connection(599,320));
        assertEquals(AdaptiveLayout.Connection.STACKED,AdaptiveLayout.connection(700,700));
        for(int width:new int[]{600,640,840,1200}) assertTrue(width-AdaptiveLayout.connectionRailDp(width)>=400);
    }
    @Test public void sheetBudgetsIntersectAvailableBoundsWithoutSubtractingTheKeyboardTwice() {
        assertEquals(288,AdaptiveLayout.sheetLimit(640,480,300,12,false));
        assertEquals(188,AdaptiveLayout.sheetLimit(200,200,200,12,false));
        assertEquals(172,AdaptiveLayout.sheetLimit(200,200,200,12,true));
        assertEquals(0,AdaptiveLayout.sheetLimit(8,300,300,12,false));
        assertEquals(288,AdaptiveLayout.sheetLimit(300,0,0,12,false));
    }
}
