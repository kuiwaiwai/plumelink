package com.kuiwaiwai.plumelink;

import android.app.Activity;
import android.os.Bundle;
import android.graphics.drawable.GradientDrawable;
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
    private LinearLayout cameraIssue;
    private TextView cameraMessage;
    private Button torchButton, cameraSettings;
    private DecoratedBarcodeView scanner;
    private boolean torch;
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }

    private void styleAction(Button button, boolean active) {
        boolean dark = AppAppearance.dark(this);
        button.setBackground(UiStyle.ripple(this, dark,
            active ? UiStyle.accent(dark) : UiStyle.accentSoft(dark), UiStyle.BUTTON_RADIUS));
        button.setTextColor(active ? UiStyle.onAccent(dark) : UiStyle.accent(dark));
        button.setActivated(active);
    }

    private Button actionButton(String label) {
        Button button = new Button(this);
        button.setText(label);
        UiStyle.button(button);
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
        boolean dark = AppAppearance.dark(this);
        getWindow().setStatusBarColor(UiStyle.background(dark));
        getWindow().setNavigationBarColor(UiStyle.background(dark));
        if (android.os.Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        getWindow().getDecorView().setSystemUiVisibility((dark ? 0 : android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR)
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
        viewport.setBackgroundColor(UiStyle.background(dark));
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
        title.setText(ui(R.string.ui_scan_the_pc_s_qr_code)); title.setTextSize(24); title.setTextColor(UiStyle.ink(dark));
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        title.setPadding(0, landscape ? 0 : dp(14), 0, landscape ? dp(4) : dp(10)); controls.addView(title);
        TextView hint = new TextView(this);
        hint.setText(ui(R.string.ui_open_plumelink_on_the_pc_and_place_its_qr_code_in_the_f));
        hint.setTextSize(14); hint.setTextColor(UiStyle.muted(dark));
        hint.setPadding(0, 0, 0, landscape ? 0 : dp(20)); controls.addView(hint);
        cameraIssue = new LinearLayout(this);
        cameraIssue.setTag("camera-issue");
        cameraIssue.setOrientation(LinearLayout.VERTICAL);
        cameraIssue.setPadding(dp(16), dp(16), dp(16), dp(16));
        cameraIssue.setBackground(UiStyle.shape(this, UiStyle.tonal(dark), UiStyle.BUTTON_RADIUS));
        cameraIssue.setVisibility(View.GONE);
        cameraMessage = new TextView(this);
        cameraMessage.setTextSize(14);
        cameraMessage.setTextColor(UiStyle.ink(dark));
        cameraMessage.setLineSpacing(dp(3), 1f);
        cameraMessage.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        cameraIssue.addView(cameraMessage);
        cameraSettings = actionButton(ui(R.string.ui_permission_settings));
        cameraSettings.setTag("camera-settings");
        cameraSettings.setOnClickListener(v -> {
            try {
                startActivity(new android.content.Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    android.net.Uri.parse("package:" + getPackageName())));
            } catch (android.content.ActivityNotFoundException ignored) {
                android.widget.Toast.makeText(this, ui(R.string.ui_allow_camera_access_in_settings_and_retry_or_go_back_an), android.widget.Toast.LENGTH_LONG).show();
            }
        });
        cameraIssue.addView(cameraSettings, actionParams(12));
        controls.addView(cameraIssue, actionParams(12));
        scanner = new DecoratedBarcodeView(this);
        GradientDrawable frame = new GradientDrawable(); frame.setColor(0xff1a202c); frame.setCornerRadius(dp(24));
        scanner.setBackground(frame); scanner.setClipToOutline(true);
        if (landscape) layout.addView(scanner, 0, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
        else layout.addView(scanner, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(MIN_PREVIEW_HEIGHT_DP), 1));
        if (getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_FLASH)) {
            Button light = actionButton(ui(R.string.ui_turn_flashlight_on));
            torchButton = light;
            light.setOnClickListener(v -> {
                torch = !torch;
                if (torch) scanner.setTorchOn(); else scanner.setTorchOff();
                light.setText(torch ? ui(R.string.ui_turn_flashlight_off) : ui(R.string.ui_turn_flashlight_on));
                styleAction(light, torch);
            });
            controls.addView(light, actionParams(landscape ? 8 : 16));
        }
        Button cancel = actionButton(ui(R.string.ui_back));
        cancel.setBackground(UiStyle.ripple(this, dark, UiStyle.tonal(dark), UiStyle.BUTTON_RADIUS));
        cancel.setTextColor(UiStyle.ink(dark));
        cancel.setOnClickListener(v -> finish());
        controls.addView(cancel, actionParams(10));
        setContentView(viewport);
        viewport.requestApplyInsets();
        capture = new CaptureManager(this, scanner) {
            @Override protected void displayFrameworkBugMessageAndExit(String message) {
                showCameraIssue();
            }
        };
        capture.setShowMissingCameraPermissionDialog(true, ui(R.string.ui_allow_camera_access_in_settings_and_retry_or_go_back_an));
        android.content.Intent scanIntent = new android.content.Intent(getIntent())
            .putExtra(com.google.zxing.client.android.Intents.Scan.ORIENTATION_LOCKED, false);
        capture.initializeFromIntent(scanIntent, saved);
        scanner.setStatusText("");
        capture.decode();
    }
    private void showCameraIssue() {
        if (isFinishing() || isDestroyed()) return;
        boolean permitted = checkSelfPermission(android.Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED;
        cameraMessage.setText(ui(permitted ? R.string.ui_could_not_open_the_camera_connect_by_address
            : R.string.ui_allow_camera_access_in_settings_and_retry_or_go_back_an));
        cameraSettings.setVisibility(permitted ? View.GONE : View.VISIBLE);
        cameraIssue.setVisibility(View.VISIBLE);
        scanner.pause();
        scanner.setVisibility(View.GONE);
        torch = false;
        if (torchButton != null) {
            torchButton.setVisibility(View.GONE);
            torchButton.setText(ui(R.string.ui_turn_flashlight_on));
            styleAction(torchButton, false);
        }
    }
    @Override protected void onResume() {
        super.onResume();
        if (checkSelfPermission(android.Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            if (cameraIssue != null) cameraIssue.setVisibility(View.GONE);
            if (scanner != null) scanner.setVisibility(View.VISIBLE);
            if (torchButton != null) torchButton.setVisibility(View.VISIBLE);
            getSharedPreferences("scanner", MODE_PRIVATE).edit().remove("camera_denied").apply();
        } else if (getSharedPreferences("scanner", MODE_PRIVATE).getBoolean("camera_denied", false)) {
            showCameraIssue();
            return;
        }
        if (capture != null) capture.onResume();
    }
    @Override protected void onPause() { if (capture != null) capture.onPause(); super.onPause(); }
    @Override protected void onDestroy() { if (capture != null) capture.onDestroy(); super.onDestroy(); }
    @Override protected void onSaveInstanceState(Bundle out) { super.onSaveInstanceState(out); if (capture != null) capture.onSaveInstanceState(out); }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == CaptureManager.getCameraPermissionReqCode() && (results.length == 0 || results[0] != android.content.pm.PackageManager.PERMISSION_GRANTED))
            getSharedPreferences("scanner", MODE_PRIVATE).edit().putBoolean("camera_denied", true).apply();
        if (capture != null) capture.onRequestPermissionsResult(request, permissions, results);
    }
}
