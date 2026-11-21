package com.kuiwaiwai.plumelink;

import android.app.Activity;
import android.os.Bundle;
import android.graphics.Color;
import android.content.res.ColorStateList;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import com.journeyapps.barcodescanner.CaptureManager;
import com.journeyapps.barcodescanner.DecoratedBarcodeView;

/** Offline QR capture with camera permission and lifecycle owned by CaptureManager. */
public final class PairScanActivity extends Activity {
    private static final int MIN_PREVIEW_HEIGHT_DP = 200;

    private String ui(int id) { return AppLanguage.text(this, id); }

    private CaptureManager capture;
    private boolean torch;
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }

    private void styleAction(Button button, boolean active) {
        int accent = Color.rgb(40, 104, 232);
        GradientDrawable background = new GradientDrawable();
        background.setColor(active ? accent : Color.rgb(237, 243, 255));
        background.setCornerRadius(dp(16));
        GradientDrawable mask = new GradientDrawable();
        mask.setColor(Color.WHITE); mask.setCornerRadius(dp(16));
        button.setBackground(new RippleDrawable(ColorStateList.valueOf(active
            ? Color.argb(45, 255, 255, 255) : Color.argb(24, 40, 104, 232)), background, mask));
        button.setTextColor(active ? Color.WHITE : accent);
        button.setActivated(active);
    }

    private Button actionButton(String label) {
        Button button = new Button(this);
        button.setText(label);
        button.setAllCaps(false);
        button.setTextSize(15);
        button.setTypeface(android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL));
        button.setLetterSpacing(0);
        button.setIncludeFontPadding(false);
        button.setGravity(Gravity.CENTER);
        button.setMinWidth(0); button.setMinimumWidth(0);
        button.setMinHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP)); button.setMinimumHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
        button.setPadding(dp(20), dp(10), dp(20), dp(10));
        button.setStateListAnimator(null);
        button.setElevation(0);
        button.setBackgroundTintList(null);
        styleAction(button, false);
        return button;
    }

    private LinearLayout.LayoutParams actionParams(int topMargin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.topMargin = dp(topMargin);
        return params;
    }

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().setStatusBarColor(Color.rgb(247, 248, 251));
        getWindow().setNavigationBarColor(Color.rgb(247, 248, 251));
        if (android.os.Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        getWindow().getDecorView().setSystemUiVisibility(android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            | android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        LinearLayout layout = new LinearLayout(this);
        boolean landscape = getResources().getConfiguration().orientation == android.content.res.Configuration.ORIENTATION_LANDSCAPE;
        layout.setOrientation(landscape ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
        View viewport = layout;
        if (!landscape) {
            ScrollView scroll = new ScrollView(this);
            scroll.setFillViewport(true);
            scroll.addView(layout);
            viewport = scroll;
        }
        viewport.setPadding(dp(22), dp(12), dp(22), dp(20));
        viewport.setOnApplyWindowInsetsListener((view, insets) -> {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(android.view.WindowInsets.Type.systemBars() | android.view.WindowInsets.Type.displayCutout());
                view.setPadding(dp(22) + bars.left, dp(12) + bars.top, dp(22) + bars.right, dp(20) + bars.bottom);
                return android.view.WindowInsets.CONSUMED;
            }
            view.setPadding(dp(22) + insets.getSystemWindowInsetLeft(), dp(12) + insets.getSystemWindowInsetTop(),
                dp(22) + insets.getSystemWindowInsetRight(), dp(20) + insets.getSystemWindowInsetBottom());
            return insets.consumeSystemWindowInsets();
        });
        viewport.setBackgroundColor(Color.rgb(247, 248, 251));
        LinearLayout controls = layout;
        if (landscape) {
            controls = new LinearLayout(this);
            controls.setOrientation(LinearLayout.VERTICAL);
            controls.setPadding(dp(16), 0, 0, 0);
            ScrollView scroll = new ScrollView(this);
            scroll.addView(controls);
            layout.addView(scroll, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
        }
        TextView title = new TextView(this);
        title.setText(ui(R.string.ui_scan_the_pc_s_qr_code)); title.setTextSize(24); title.setTextColor(Color.rgb(27, 35, 49));
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        title.setPadding(0, landscape ? 0 : dp(14), 0, landscape ? dp(4) : dp(10)); controls.addView(title);
        TextView hint = new TextView(this);
        hint.setText(ui(R.string.ui_open_plumelink_on_the_pc_and_place_its_qr_code_in_the_f));
        hint.setTextSize(14); hint.setTextColor(Color.rgb(107, 118, 136));
        hint.setPadding(0, 0, 0, landscape ? 0 : dp(20)); controls.addView(hint);
        DecoratedBarcodeView scanner = new DecoratedBarcodeView(this);
        GradientDrawable frame = new GradientDrawable(); frame.setColor(Color.rgb(27, 35, 49)); frame.setCornerRadius(dp(24));
        scanner.setBackground(frame); scanner.setClipToOutline(true);
        if (landscape) layout.addView(scanner, 0, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
        else layout.addView(scanner, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(MIN_PREVIEW_HEIGHT_DP), 1));
        if (getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_FLASH)) {
            Button light = actionButton(ui(R.string.ui_turn_flashlight_on));
            light.setOnClickListener(v -> {
                torch = !torch;
                if (torch) scanner.setTorchOn(); else scanner.setTorchOff();
                light.setText(torch ? ui(R.string.ui_turn_flashlight_off) : ui(R.string.ui_turn_flashlight_on));
                styleAction(light, torch);
            });
            controls.addView(light, actionParams(landscape ? 8 : 16));
        }
        Button cancel = actionButton(ui(R.string.ui_back));
        GradientDrawable cancelBackground = new GradientDrawable();
        cancelBackground.setColor(Color.WHITE); cancelBackground.setCornerRadius(dp(16));
        GradientDrawable cancelMask = new GradientDrawable();
        cancelMask.setColor(Color.WHITE); cancelMask.setCornerRadius(dp(16));
        cancel.setBackground(new RippleDrawable(ColorStateList.valueOf(Color.argb(24, 40, 104, 232)), cancelBackground, cancelMask));
        cancel.setTextColor(Color.rgb(27, 35, 49));
        cancel.setOnClickListener(v -> finish());
        controls.addView(cancel, actionParams(10));
        setContentView(viewport);
        viewport.requestApplyInsets();
        capture = new CaptureManager(this, scanner);
        capture.setShowMissingCameraPermissionDialog(true, ui(R.string.ui_allow_camera_access_in_settings_and_retry_or_go_back_an));
        capture.initializeFromIntent(getIntent(), saved);
        scanner.setStatusText("");
        capture.decode();
    }
    @Override protected void onResume() { super.onResume(); if (capture != null) capture.onResume(); }
    @Override protected void onPause() { if (capture != null) capture.onPause(); super.onPause(); }
    @Override protected void onDestroy() { if (capture != null) capture.onDestroy(); super.onDestroy(); }
    @Override protected void onSaveInstanceState(Bundle out) { super.onSaveInstanceState(out); if (capture != null) capture.onSaveInstanceState(out); }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (capture != null) capture.onRequestPermissionsResult(request, permissions, results);
    }
}
