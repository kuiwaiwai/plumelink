package com.kuiwaiwai.plumelink;

final class AdaptiveLayout {
    enum Workspace { STACKED, NAVIGATION_RAIL, TWO_PANE }

    static final int SHEET_MAX_WIDTH_DP = 560;
    static final int TOUCH_TARGET_DP = 48;
    static final int NAV_RAIL_MIN_WIDTH_DP = 80;
    static final int NAV_RAIL_MAX_WIDTH_DP = 140;
    private static final int MEDIUM_WIDTH_DP = 600;
    private static final int EXPANDED_WIDTH_DP = 840;
    private static final int MEDIUM_HEIGHT_DP = 480;

    private AdaptiveLayout() {}

    static Workspace workspace(int widthDp, int heightDp) {
        if (widthDp < MEDIUM_WIDTH_DP || widthDp <= heightDp) return Workspace.STACKED;
        if (widthDp >= EXPANDED_WIDTH_DP && heightDp >= MEDIUM_HEIGHT_DP) return Workspace.TWO_PANE;
        return Workspace.NAVIGATION_RAIL;
    }

    static int pageGutterDp(int widthDp, Workspace workspace) {
        if (workspace == Workspace.TWO_PANE) return Math.max(20, Math.min(40, Math.round(widthDp * .025f)));
        return widthDp < 360 ? 12 : 16;
    }

    static boolean fitsInline(int availableWidth, float leadingWidth, float trailingWidth, int gap) {
        return leadingWidth + trailingWidth + gap <= availableWidth;
    }

    static int dialDiameterDp(int available, int requested) {
        int limit = Math.max(0, available);
        int step = Math.min(12, Math.max(0, (limit - 152) / 2));
        int largerSizes = requested <= 176 ? 2 : requested <= 208 ? 1 : 0;
        return Math.min(requested, limit - largerSizes * step);
    }

    static int fittingColumns(int availableWidth, int gap, int minCellWidth, int maxColumns) {
        for (int count = maxColumns; count > 1; count--) {
            if (count * minCellWidth + (count - 1) * gap <= availableWidth) return count;
        }
        return 1;
    }
}
