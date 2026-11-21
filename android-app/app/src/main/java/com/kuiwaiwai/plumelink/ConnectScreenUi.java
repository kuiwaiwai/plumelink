package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.Dialog;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.os.Build;
import android.provider.Settings;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageButton;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.RadioButton;
import android.widget.RadioGroup;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

final class ConnectScreenUi {
    interface Host {
        void onConnectRequested(String url);
        void onScanRequested();
        void onLanguageSelected(String value);
        void onRefreshScreen(String initial, String errorMessage);
        boolean isWebViewActive();
    }

    private boolean dark;
    private int background, surface, ink, muted, availableColor, tile, edge, accent, onAccent, dangerInk, dangerSurface;

    private void updatePalette() {
        dark = AppAppearance.dark(activity);
        background = dark ? 0xff111318 : 0xfff7f8fb;
        surface = dark ? 0xff1c1f27 : Color.WHITE;
        ink = dark ? 0xffeceef5 : 0xff1b2331;
        muted = dark ? 0xffafb8ca : 0xff637084;
        availableColor = dark ? 0xff6bd5ae : 0xff16714f;
        tile = dark ? 0xff293958 : 0xffedf3ff;
        edge = dark ? 0xff343947 : 0xffe6eaf2;
        accent = dark ? 0xffa5bdff : 0xff2868e8;
        onAccent = dark ? 0xff122448 : Color.WHITE;
        dangerInk = dark ? 0xffffb1b6 : 0xffae3039;
        dangerSurface = dark ? 0xff362127 : 0xfffff5f5;
    }
    private static final int SAVED_PC_REFRESH_MS = 4000;
    private static final int MAX_PROBE_WORKERS = 4;

    private final Activity activity;
    private final FrameLayout root;
    private final ComputerStore store;
    private final LanDiscovery discovery;
    private final Host host;

    private LinearLayout discoveryPanel;
    private final Map<String, TextView> savedStatuses = new LinkedHashMap<>();
    private final ExecutorService probes = Executors.newFixedThreadPool(MAX_PROBE_WORKERS);
    private final Runnable refreshSaved = this::probeSavedComputers;
    private volatile int generation;
    private int runningProbes;
    private boolean foreground;
    private boolean autoConnectLast;

    ConnectScreenUi(Activity activity, FrameLayout root, ComputerStore store, LanDiscovery discovery, Host host) {
        this.activity = activity;
        this.root = root;
        this.store = store;
        this.discovery = discovery;
        this.host = host;
        updatePalette();
    }

    void enableAutoConnectLast(boolean enabled) { this.autoConnectLast = enabled; }
    void setForeground(boolean foreground) {
        this.foreground = foreground;
        if (foreground) refreshSavedComputers();
        else stopRefreshing();
    }

    private void stopRefreshing() {
        generation++;
        root.removeCallbacks(refreshSaved);
    }

    void hide() {
        stopRefreshing();
        savedStatuses.clear();
        discoveryPanel = null;
    }

    void destroy() {
        hide();
        probes.shutdownNow();
    }

    private String ui(int id) { return AppLanguage.text(activity, id); }
    int dp(int value) { return Math.round(value * activity.getResources().getDisplayMetrics().density); }

    private TextView text(String value, int size, int color) {
        TextView view = new TextView(activity);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        return view;
    }

    private GradientDrawable rounded(int color, int radius, int strokeColor) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(color);
        drawable.setCornerRadius(dp(radius));
        if (strokeColor != Color.TRANSPARENT) drawable.setStroke(dp(1), strokeColor);
        return drawable;
    }

    private RippleDrawable ripple(int color, int radius) {
        return new RippleDrawable(ColorStateList.valueOf(Color.argb(34, 104, 125, 170)),
            rounded(color, radius, Color.TRANSPARENT), rounded(Color.WHITE, radius, Color.TRANSPARENT));
    }

    private LinearLayout sheetContent() {
        LinearLayout sheet = new LinearLayout(activity);
        sheet.setOrientation(LinearLayout.VERTICAL);
        sheet.setPadding(dp(24), dp(24), dp(24), dp(28));
        GradientDrawable background = new GradientDrawable();
        background.setColor(surface);
        background.setCornerRadii(new float[]{dp(28), dp(28), dp(28), dp(28), 0, 0, 0, 0});
        sheet.setBackground(background);
        return sheet;
    }

    private void presentSheet(Dialog dialog, View content) {
        presentSheet(dialog, content, false);
    }

    private void presentSheet(Dialog dialog, View content, boolean reading) {
        dialog.setContentView(content);
        Window window = dialog.getWindow();
        if (window != null) {
            window.setBackgroundDrawableResource(android.R.color.transparent);
            window.setGravity(Gravity.BOTTOM);
            window.addFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND);
            window.setDimAmount(0.38f);
            window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        }
        dialog.show();
        if (window != null) {
            Runnable resize = () -> window.setLayout(
                root.getWidth() > dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP)
                    ? dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP) : ViewGroup.LayoutParams.MATCH_PARENT,
                reading && root.getHeight() > 0 ? Math.round(root.getHeight() * 0.86f) : ViewGroup.LayoutParams.WRAP_CONTENT);
            resize.run();
            View.OnLayoutChangeListener listener = (v, l, t, r, b, oldL, oldT, oldR, oldB) -> resize.run();
            root.addOnLayoutChangeListener(listener);
            dialog.setOnDismissListener(ignored -> root.removeOnLayoutChangeListener(listener));
        }
    }

    AlertDialog showSecurityDialog(AlertDialog.Builder builder) {
        AlertDialog dialog = builder.create();
        dialog.setOnShowListener(ignored -> {
            if (dialog.getWindow() != null) dialog.getWindow().setBackgroundDrawable(rounded(surface, 24, Color.TRANSPARENT));
            if (dialog.getButton(AlertDialog.BUTTON_POSITIVE) != null) dialog.getButton(AlertDialog.BUTTON_POSITIVE).setTextColor(accent);
            if (dialog.getButton(AlertDialog.BUTTON_NEGATIVE) != null) dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setTextColor(muted);
        });
        dialog.show();
        return dialog;
    }

    @SuppressLint("SetTextI18n")
    private String shownAddress = "", shownError = "";
    boolean themeChanged() { return dark != AppAppearance.dark(activity); }
    void refreshPalette() { show(shownAddress, shownError); }

    void show(String initial, String errorMessage) {
        shownAddress = initial;
        shownError = errorMessage;
        updatePalette();
        hide();
        root.removeAllViews();
        root.setBackgroundColor(background);

        LinearLayout panel = new LinearLayout(activity);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(dp(20), dp(16), dp(20), dp(16));

        LinearLayout brand = new LinearLayout(activity);
        brand.setOrientation(LinearLayout.HORIZONTAL);
        brand.setGravity(Gravity.CENTER_VERTICAL);
        ImageView logo = new ImageView(activity);
        logo.setImageResource(R.drawable.ic_launcher);
        brand.addView(logo, new LinearLayout.LayoutParams(dp(36), dp(36)));
        LinearLayout brandText = new LinearLayout(activity);
        brandText.setOrientation(LinearLayout.VERTICAL);
        brandText.setPadding(dp(8), 0, 0, 0);
        TextView appName = text(ui(R.string.app_name), 15, ink);
        appName.setTypeface(null, android.graphics.Typeface.BOLD);
        brandText.addView(appName);
        brand.addView(brandText, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));

        TextView title = text(ui(R.string.ui_my_pcs), 28, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        if (Build.VERSION.SDK_INT >= 28) title.setAccessibilityHeading(true);
        title.setPadding(0, dp(24), 0, 0);
        TextView subtitle = text(ui(R.string.ui_select_a_saved_pc_or_find_one_nearby), 14, muted);
        subtitle.setPadding(0, dp(8), 0, dp(16));

        panel.addView(brand);
        panel.addView(title);
        panel.addView(subtitle);
        if (!errorMessage.isEmpty()) {
            TextView error = text(errorMessage, 13, dangerInk);
            error.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
            error.setPadding(dp(14), dp(11), dp(14), dp(11));
            error.setBackground(rounded(dangerSurface, 16, edge));
            LinearLayout.LayoutParams errorParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            errorParams.setMargins(0, 0, 0, dp(14));
            panel.addView(error, errorParams);
            error.post(() -> error.requestRectangleOnScreen(
                new android.graphics.Rect(0, 0, error.getWidth(), error.getHeight()), true));
        }

        ArrayList<String> savedComputers = store.visibleSavedComputers();
        if (!savedComputers.isEmpty()) {
            panel.addView(sectionTitle(ui(R.string.ui_recent_pcs)));
            LinearLayout savedCard = card();
            int pinnedCount = 0;
            for (String candidate : savedComputers) if (store.isPinned(candidate)) pinnedCount++;
            for (int index = 0; index < savedComputers.size(); index++) {
                String savedUrl = savedComputers.get(index);
                boolean pinned = store.isPinned(savedUrl);
                int groupIndex = pinned ? index : index - pinnedCount;
                int groupCount = pinned ? pinnedCount : savedComputers.size() - pinnedCount;
                ComputerRow savedRow = computerRow(store.computerName(savedUrl), store.computerSubtitle(savedUrl), ui(R.string.ui_checking), muted);
                addSavedControls(savedRow, savedUrl, groupIndex, groupCount);
                savedRow.root.setOnClickListener(v -> host.onConnectRequested(savedUrl));
                savedRow.root.setOnLongClickListener(v -> {
                    showSavedComputerActions(savedUrl, groupIndex, groupCount);
                    return true;
                });
                if (index > 0) savedCard.addView(divider());
                savedCard.addView(savedRow.root);
                savedStatuses.put(savedUrl, savedRow.status);
            }
            panel.addView(savedCard, cardParams(dp(16)));
        }

        LinearLayout nearbyHeader = new LinearLayout(activity);
        nearbyHeader.setOrientation(LinearLayout.HORIZONTAL);
        nearbyHeader.setGravity(Gravity.CENTER_VERTICAL);
        TextView nearbyTitle = sectionTitle(ui(R.string.ui_nearby_pcs));
        nearbyTitle.setPadding(0, 0, 0, 0);
        nearbyHeader.addView(nearbyTitle, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        TextView refresh = text(ui(R.string.ui_search_again), 13, accent);
        refresh.setTypeface(null, android.graphics.Typeface.BOLD);
        refresh.setGravity(Gravity.CENTER);
        refresh.setMinimumHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
        refresh.setMinimumWidth(dp(AdaptiveLayout.TOUCH_TARGET_DP));
        refresh.setPadding(dp(12), 0, 0, 0);
        refresh.setOnClickListener(v -> {
            discovery.retry();
            refreshSavedComputers();
        });
        nearbyHeader.addView(refresh);
        panel.addView(nearbyHeader);

        discoveryPanel = card();
        panel.addView(discoveryPanel, cardParams(dp(18)));
        renderDiscoveredComputers();

        ScrollView scroll = new ScrollView(activity);
        scroll.setFillViewport(true);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.addView(panel, new ScrollView.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        LinearLayout screen = new LinearLayout(activity);
        screen.setOrientation(LinearLayout.VERTICAL);
        root.addView(screen, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        screen.addView(scroll, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1));
        refreshSavedComputers();

        LinearLayout dock = new LinearLayout(activity);
        dock.setOrientation(LinearLayout.VERTICAL);
        dock.setPadding(dp(20), dp(16), dp(20), dp(16));
        dock.setBackground(rounded(surface, 0, Color.TRANSPARENT));
        dock.setElevation(dp(8));
        Button manual = new Button(activity);
        manual.setAllCaps(false);
        manual.setText(ui(R.string.ui_scan_to_connect));
        manual.setTextColor(onAccent);
        manual.setTextSize(16);
        manual.setTypeface(null, android.graphics.Typeface.BOLD);
        manual.setMinHeight(dp(54));
        manual.setPadding(0, 0, 0, 0);
        manual.setBackground(ripple(accent, 16));
        manual.setOnClickListener(v -> host.onScanRequested());
        dock.addView(manual, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        LinearLayout dockMeta = new LinearLayout(activity);
        dockMeta.setOrientation(LinearLayout.HORIZONTAL);
        dockMeta.setGravity(Gravity.CENTER_VERTICAL);
        dockMeta.setMinimumHeight(dp(48));
        LinearLayout.LayoutParams metaParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        metaParams.setMargins(0, dp(4), 0, 0);
        dockMeta.addView(dockAction(R.string.ui_connect_by_address, accent, () -> showManualConnectSheet(initial)),
            new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        dockMeta.addView(dockAction(R.string.ui_connection_help, muted, this::showConnectionHelp),
            new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        dockMeta.addView(dockAction(R.string.ui_privacy_policy, muted, this::showPrivacyPolicy),
            new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        dock.addView(dockMeta, metaParams);
        screen.addView(dock, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        screen.addOnLayoutChangeListener((v, l, t, r, b, oldL, oldT, oldR, oldB) -> {
            float density = activity.getResources().getDisplayMetrics().density;
            boolean wide = AdaptiveLayout.workspace(Math.round((r - l) / density), Math.round((b - t) / density))
                == AdaptiveLayout.Workspace.TWO_PANE;
            int orientation = wide ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL;
            if (screen.getOrientation() == orientation) return;
            screen.setOrientation(orientation);
            int fill = ViewGroup.LayoutParams.MATCH_PARENT;
            scroll.setLayoutParams(wide ? new LinearLayout.LayoutParams(0, fill, 1) : new LinearLayout.LayoutParams(fill, 0, 1));
            dock.setLayoutParams(new LinearLayout.LayoutParams(wide ? dp(304) : fill, wide ? fill : ViewGroup.LayoutParams.WRAP_CONTENT));
            dock.setGravity(wide ? Gravity.CENTER_VERTICAL : Gravity.TOP);
        });
    }

    private Button dockAction(int label, int color, Runnable action) {
        Button button = new Button(activity);
        button.setText(ui(label));
        button.setTextColor(color);
        button.setTextSize(13);
        button.setAllCaps(false);
        button.setMinimumHeight(dp(48));
        button.setPadding(dp(8), dp(6), dp(8), dp(6));
        button.setStateListAnimator(null);
        button.setBackground(ripple(Color.TRANSPARENT, 14));
        button.setOnClickListener(v -> action.run());
        return button;
    }

    void renderDiscoveredComputers() {
        if (discoveryPanel == null) return;
        discoveryPanel.removeAllViews();
        Map<String, String> computers = discovery.computers();
        String discoveryError = discovery.error();
        if (computers.isEmpty()) {
            LinearLayout empty = new LinearLayout(activity);
            empty.setOrientation(LinearLayout.HORIZONTAL);
            empty.setGravity(Gravity.CENTER_VERTICAL);
            empty.setPadding(dp(12), dp(16), dp(12), dp(16));
            FrameLayout iconBox = new FrameLayout(activity);
            iconBox.setBackground(rounded(tile, 13, Color.TRANSPARENT));
            if (!discovery.isFinished() && discoveryError.isEmpty()) {
                ProgressBar progress = new ProgressBar(activity);
                progress.setIndeterminateTintList(ColorStateList.valueOf(accent));
                iconBox.addView(progress, new FrameLayout.LayoutParams(dp(23), dp(23), Gravity.CENTER));
            } else {
                iconBox.addView(icon(R.drawable.ic_computer_outline, dp(24)), new FrameLayout.LayoutParams(dp(24), dp(24), Gravity.CENTER));
            }
            empty.addView(iconBox, new LinearLayout.LayoutParams(dp(48), dp(48)));
            LinearLayout copy = new LinearLayout(activity);
            copy.setOrientation(LinearLayout.VERTICAL);
            LinearLayout.LayoutParams copyParams = new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1);
            copyParams.setMargins(dp(12), 0, 0, 0);
            TextView heading = text(!discoveryError.isEmpty() ? ui(R.string.ui_search_unavailable)
                : discovery.isFinished() ? ui(R.string.ui_no_pcs_found_nearby) : ui(R.string.ui_searching_for_nearby_pcs), 15, ink);
            heading.setTypeface(null, android.graphics.Typeface.BOLD);
            copy.addView(heading);
            TextView caption = text(!discoveryError.isEmpty() ? discoveryError
                : discovery.isFinished() ? ui(R.string.ui_use_the_address_option_below_to_connect) : ui(R.string.ui_check_that_remote_control_is_running_on_the_pc), 12, muted);
            caption.setPadding(0, dp(4), 0, 0);
            copy.addView(caption);
            empty.addView(copy, copyParams);
            discoveryPanel.addView(empty);
        } else {
            int index = 0;
            for (Map.Entry<String, String> entry : computers.entrySet()) {
                String url = entry.getKey();
                String identity = discovery.identityFor(url);
                String known = identity.isEmpty() ? "" : store.knownComputerUrl(identity);
                ComputerRow row = computerRow(
                    known.isEmpty() ? ui(R.string.ui_nearby_windows_pc) : ui(R.string.ui_previously_connected_pc),
                    url.replace("https://", ""),
                    known.isEmpty() ? ui(R.string.ui_new_device) : known.equals(url) ? ui(R.string.ui_recognized) : ui(R.string.ui_address_updated),
                    accent);
                row.root.setOnClickListener(v -> host.onConnectRequested(url));
                if (index++ > 0) discoveryPanel.addView(divider());
                discoveryPanel.addView(row.root);
            }
        }
    }

    void refreshSavedComputers() {
        root.removeCallbacks(refreshSaved);
        probeSavedComputers();
    }

    private void probeSavedComputers() {
        if (!foreground || host.isWebViewActive() || savedStatuses.isEmpty() || runningProbes != 0) return;
        int targetGen = generation;
        ArrayList<String> urls = new ArrayList<>(savedStatuses.keySet());
        int workers = Math.min(MAX_PROBE_WORKERS, urls.size());
        runningProbes = workers;
        for (int worker = 0; worker < workers; worker++) {
            int start = worker;
            probes.execute(() -> {
                try {
                    for (int i = start; i < urls.size() && targetGen == generation; i += workers) {
                        String url = urls.get(i);
                        int status = savedServiceStatus(url);
                        activity.runOnUiThread(() -> applySavedStatus(targetGen, url, status));
                    }
                } finally {
                    activity.runOnUiThread(() -> {
                        if (--runningProbes != 0 || !foreground || savedStatuses.isEmpty()) return;
                        root.postDelayed(refreshSaved, targetGen == generation ? SAVED_PC_REFRESH_MS : 0);
                    });
                }
            });
        }
    }

    private int savedServiceStatus(String url) {
        if (!EndpointUtils.isValidPrivateEndpoint(url)) return R.string.ui_unreachable;
        String hostName = android.net.Uri.parse(url).getHost();
        String pin = store.trustedPin(hostName, store.storedIdentityOrEmpty(url));
        if (!pin.isEmpty()) {
            javax.net.ssl.HttpsURLConnection connection = null;
            try {
                // The paired service requires authentication on this route. Never send a saved token in a background probe.
                connection = new PinnedConnections(hostName, pin).open(url + "/api/status", "");
                connection.setConnectTimeout(1400);
                connection.setReadTimeout(1400);
                if (connection.getResponseCode() == 401) return R.string.ui_available;
            } catch (Exception ignored) {
                // A listening address can still be starting up, or its certificate may have changed.
            } finally {
                if (connection != null) connection.disconnect();
            }
        }
        return EndpointUtils.isReachable(url) ? R.string.ui_service_unconfirmed : R.string.ui_unreachable;
    }

    private void applySavedStatus(int targetGen, String url, int result) {
        if (targetGen != generation || !foreground || host.isWebViewActive()) return;
        TextView status = savedStatuses.get(url);
        if (status == null) return;
        boolean available = result == R.string.ui_available;
        String label = ui(result);
        if (!label.contentEquals(status.getText())) {
            status.setText(label);
            status.setTextColor(available ? availableColor : muted);
        }
        if (autoConnectLast && url.equals(store.lastUrl())) {
            autoConnectLast = false;
            if (available) host.onConnectRequested(url);
        }
    }

    private void showManualConnectSheet(String initial) {
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView title = text(ui(R.string.ui_enter_pc_address), 23, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(title);
        TextView description = text(ui(R.string.ui_find_the_address_below_the_qr_code_on_the_windows_pc), 14, muted);
        description.setPadding(0, dp(8), 0, dp(22));
        sheet.addView(description);
        EditText address = new EditText(activity);
        address.setHint(String.format(java.util.Locale.ROOT, ui(R.string.ui_pc_address_example), EndpointUtils.DEFAULT_SERVICE_PORT));
        address.setSingleLine(true);
        address.setTextColor(ink);
        address.setHintTextColor(muted);
        address.setTextSize(16);
        address.setPadding(dp(16), 0, dp(16), 0);
        address.setBackground(rounded(background, 14, edge));
        address.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_VARIATION_URI);
        address.setImeOptions(EditorInfo.IME_ACTION_GO);
        if (!initial.isEmpty()) address.setText(initial.replace("https://", ""));
        address.setMinimumHeight(dp(54));
        sheet.addView(address, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        Button submit = new Button(activity);
        submit.setAllCaps(false);
        submit.setText(ui(R.string.ui_connect_to_pc));
        submit.setTextSize(16);
        submit.setTypeface(null, android.graphics.Typeface.BOLD);
        submit.setTextColor(onAccent);
        submit.setMinHeight(dp(52));
        submit.setBackground(ripple(accent, 15));
        LinearLayout.LayoutParams submitParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        submitParams.setMargins(0, dp(16), 0, 0);
        submit.setOnClickListener(v -> {
            String normalized = EndpointUtils.normalize(address.getText().toString());
            if (!EndpointUtils.isValidPrivateEndpoint(normalized)) {
                address.setError(ui(R.string.ui_enter_a_reachable_address_on_your_local_or_private_netw));
                return;
            }
            dialog.dismiss();
            host.onConnectRequested(normalized);
        });
        address.setOnEditorActionListener((view, action, event) -> {
            if (event == null) {
                if (action != EditorInfo.IME_ACTION_GO) return false;
                submit.performClick();
                return true;
            }
            int key = event.getKeyCode();
            if (key != KeyEvent.KEYCODE_ENTER && key != KeyEvent.KEYCODE_NUMPAD_ENTER) return false;
            if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) submit.performClick();
            return true;
        });
        sheet.addView(submit, submitParams);
        ScrollView scroll = new ScrollView(activity);
        scroll.addView(sheet);
        presentSheet(dialog, scroll);
    }

    private void showConnectionHelp() {
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView title = text(ui(R.string.ui_connection_help), 23, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(title);
        TextView instructions = text(ui(R.string.ui_1_start_remote_control_on_the_windows_pc_2_join_the_sam), 14, muted);
        instructions.setLineSpacing(dp(5), 1f);
        instructions.setPadding(0, dp(17), 0, dp(14));
        sheet.addView(instructions);
        TextView security = text(ui(R.string.ui_the_connection_is_encrypted_use_a_trusted_private_netwo), 13, muted);
        security.setPadding(0, 0, 0, dp(12));
        sheet.addView(security);
        addComputerAction(sheet, ui(R.string.ui_language), true, false, () -> {
            dialog.dismiss();
            showLanguagePicker();
        });
        if (Build.VERSION.SDK_INT >= 33 && activity.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            TextView notifyLink = text(ui(R.string.ui_enable_notifications_for_background_transfers_and_micro), 14, accent);
            notifyLink.setTypeface(null, android.graphics.Typeface.BOLD);
            notifyLink.setPadding(0, dp(8), 0, dp(8));
            notifyLink.setOnClickListener(v -> {
                dialog.dismiss();
                try {
                    Intent settingsIntent = new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, activity.getPackageName());
                    activity.startActivity(settingsIntent);
                } catch (Exception ignored) {}
            });
            sheet.addView(notifyLink);
        }
        if (store.hasTrustRecord()) {
            TextView reset = text(ui(R.string.ui_reset_trusted_certificates), 14, accent);
            reset.setTypeface(null, android.graphics.Typeface.BOLD);
            reset.setGravity(Gravity.CENTER_VERTICAL);
            reset.setPadding(0, dp(10), 0, dp(10));
            reset.setOnClickListener(v -> {
                dialog.dismiss();
                showSecurityDialog(AppAppearance.dialog(activity)
                    .setTitle(ui(R.string.ui_reset_trusted_certificates_label))
                    .setMessage(ui(R.string.ui_use_this_after_reinstalling_the_pc_or_changing_its_cert))
                    .setNegativeButton(ui(R.string.ui_cancel), null)
                    .setPositiveButton(ui(R.string.ui_reset_trust), (confirm, which) -> {
                        store.resetTrustedCertificates();
                        Toast.makeText(activity, ui(R.string.ui_trusted_certificates_reset), Toast.LENGTH_SHORT).show();
                    }));
            });
            sheet.addView(reset);
        }
        ScrollView scroll = new ScrollView(activity);
        scroll.addView(sheet);
        presentSheet(dialog, scroll);
    }

    private void showLanguagePicker() {
        Dialog dialog = new Dialog(activity);
        RadioGroup choices = new RadioGroup(activity);
        String[] labels = {ui(R.string.ui_system_default), "简体中文", "繁體中文", "English"};
        String current = AppLanguage.preference(activity);
        for (int i = 0; i < labels.length; i++) {
            String value = AppLanguage.OPTIONS[i];
            boolean selected = value.equals(current);
            RadioButton option = new RadioButton(activity);
            option.setId(View.generateViewId());
            option.setText(labels[i]);
            option.setTextSize(16);
            option.setTextColor(selected ? accent : ink);
            option.setButtonTintList(ColorStateList.valueOf(selected ? accent : muted));
            option.setBackground(ripple(selected ? tile : background, 14));
            option.setPadding(dp(12), dp(8), dp(12), dp(8));
            option.setMinimumHeight(dp(52));
            RadioGroup.LayoutParams params = new RadioGroup.LayoutParams(-1, -2);
            params.topMargin = i == 0 ? 0 : dp(8);
            choices.addView(option, params);
            if (selected) choices.check(option.getId());
            option.setOnClickListener(v -> {
                dialog.dismiss();
                host.onLanguageSelected(value);
                host.onRefreshScreen("", "");
            });
        }
        presentInfoSheet(dialog, R.string.ui_language, choices, false);
    }

    void showPrivacyPolicy() {
        updatePalette();
        TextView body = text("", 15, ink);
        body.setText(PrivacyPolicy.content(activity));
        body.setLineSpacing(dp(4), 1f);
        body.setTextIsSelectable(true);
        presentInfoSheet(new Dialog(activity), R.string.ui_privacy_policy, body, true);
    }

    private void presentInfoSheet(Dialog dialog, int titleId, View content, boolean reading) {
        LinearLayout sheet = sheetContent();
        sheet.setPadding(dp(24), dp(12), dp(24), dp(20));
        LinearLayout header = new LinearLayout(activity);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text(ui(titleId), 22, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        if (Build.VERSION.SDK_INT >= 28) title.setAccessibilityHeading(true);
        header.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        ImageButton close = new ImageButton(activity);
        close.setImageDrawable(NativeIcon.of("close", muted, dp(20)));
        close.setPadding(dp(14), dp(14), dp(14), dp(14));
        close.setBackground(ripple(Color.TRANSPARENT, 24));
        close.setContentDescription(ui(R.string.ui_close_panel));
        close.setOnClickListener(v -> dialog.dismiss());
        header.addView(close, new LinearLayout.LayoutParams(dp(48), dp(48)));
        sheet.addView(header);
        ScrollView scroll = new ScrollView(activity);
        scroll.addView(content);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, reading ? 0 : -2, reading ? 1 : 0);
        params.topMargin = dp(8);
        sheet.addView(scroll, params);
        dialog.setTitle(ui(titleId));
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE);
        dialog.setCanceledOnTouchOutside(true);
        presentSheet(dialog, sheet, reading);
    }

    private void showSavedComputerActions(String url, int index, int count) {
        autoConnectLast = false;
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView heading = text(store.computerName(url), 21, ink);
        heading.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(heading);
        TextView address = text(store.computerSubtitle(url), 13, muted);
        address.setTextDirection(View.TEXT_DIRECTION_LTR);
        address.setPadding(0, dp(6), 0, dp(12));
        sheet.addView(address);
        addComputerAction(sheet, ui(R.string.ui_edit_name_and_location), true, false, () -> { dialog.dismiss(); showComputerDetailsEditor(url); });
        boolean pinned = store.isPinned(url);
        addComputerAction(sheet, pinned ? ui(R.string.ui_unpin) : ui(R.string.ui_pin_pc), true, false, () -> {
            dialog.dismiss(); store.togglePinned(url); host.onRefreshScreen("", "");
        });
        addComputerAction(sheet, ui(R.string.ui_move_up), index > 0, false, () -> {
            dialog.dismiss(); store.moveSavedComputer(url, -1); host.onRefreshScreen("", "");
        });
        addComputerAction(sheet, ui(R.string.ui_move_down), index + 1 < count, false, () -> {
            dialog.dismiss(); store.moveSavedComputer(url, 1); host.onRefreshScreen("", "");
        });
        String identity = store.identityForUrl(url);
        if (!store.token(identity).isEmpty()) {
            addComputerAction(sheet, ui(R.string.ui_pair_again), true, false, () -> {
                dialog.dismiss();
                AppAppearance.dialog(activity).setTitle(ui(R.string.ui_pair_again))
                    .setMessage(store.computerName(url) + "\n\n" + ui(R.string.ui_pair_again_explanation))
                    .setNegativeButton(ui(R.string.ui_cancel), null)
                    .setPositiveButton(ui(R.string.ui_pair_again), (ignored, which) -> {
                        store.setToken(identity, "");
                        host.onConnectRequested(url);
                    }).show();
            });
        }
        addComputerAction(sheet, ui(R.string.ui_remove_this_pc), true, true, () -> {
            dialog.dismiss();
            AppAppearance.dialog(activity).setTitle(ui(R.string.ui_remove_this_pc_label))
                .setMessage(store.computerName(url) + ui(R.string.ui_will_be_removed_from_saved_pcs_along_with_its_pairing_c))
                .setNegativeButton(ui(R.string.ui_cancel), null)
                .setPositiveButton(ui(R.string.ui_remove), (ignored, which) -> {
                    store.forgetComputer(url);
                    host.onRefreshScreen("", ui(R.string.ui_pc_removed));
                }).show();
        });
        addComputerAction(sheet, ui(R.string.ui_cancel), true, false, dialog::dismiss);
        ScrollView scroll = new ScrollView(activity);
        scroll.setFillViewport(false);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.addView(sheet);
        presentSheet(dialog, scroll);
    }

    private void showComputerDetailsEditor(String url) {
        LinearLayout form = new LinearLayout(activity);
        form.setOrientation(LinearLayout.VERTICAL);
        form.setPadding(dp(22), dp(8), dp(22), 0);
        EditText alias = new EditText(activity);
        alias.setTextColor(ink);
        alias.setHintTextColor(muted);
        alias.setBackgroundTintList(ColorStateList.valueOf(muted));
        alias.setSingleLine(true);
        alias.setHint(ui(R.string.ui_name_e_g_study_pc));
        alias.setText(store.computerMetadata(url, ComputerStore.ALIAS_PREFIX));
        alias.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(40)});
        alias.setMinimumHeight(dp(54));
        form.addView(alias, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        EditText location = new EditText(activity);
        location.setTextColor(ink);
        location.setHintTextColor(muted);
        location.setBackgroundTintList(ColorStateList.valueOf(muted));
        location.setSingleLine(true);
        location.setHint(ui(R.string.ui_location_e_g_upstairs_study));
        location.setText(store.computerMetadata(url, ComputerStore.LOCATION_PREFIX));
        location.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(60)});
        location.setMinimumHeight(dp(54));
        form.addView(location, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        ScrollView formScroll = new ScrollView(activity);
        formScroll.addView(form);
        AlertDialog dialog = AppAppearance.dialog(activity).setTitle(ui(R.string.ui_pc_name_and_location)).setView(formScroll)
            .setPositiveButton(ui(R.string.ui_save), (d, which) -> {
                store.saveComputerMetadata(url, alias.getText().toString(), location.getText().toString());
                host.onRefreshScreen("", ui(R.string.ui_pc_details_saved));
            })
            .setNeutralButton(ui(R.string.ui_clear), (d, which) -> {
                store.saveComputerMetadata(url, "", "");
                host.onRefreshScreen("", ui(R.string.ui_pc_details_cleared));
            })
            .setNegativeButton(ui(R.string.ui_cancel), null).create();
        dialog.setOnShowListener(ignored -> {
            if (dialog.getWindow() != null) dialog.getWindow().setBackgroundDrawable(rounded(surface, 24, Color.TRANSPARENT));
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setTextColor(accent);
            dialog.getButton(AlertDialog.BUTTON_NEUTRAL).setTextColor(accent);
            dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setTextColor(muted);
        });
        dialog.show();
    }

    private void addSavedControls(ComputerRow row, String url, int index, int count) {
        if (store.isPinned(url)) {
            TextView pinned = text(ui(R.string.ui_pinned), 11, accent);
            pinned.setPadding(dp(7), dp(1), dp(7), dp(1));
            pinned.setBackground(rounded(tile, 6, Color.TRANSPARENT));
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            params.setMargins(dp(8), 0, 0, 0);
            row.metadata.addView(pinned, params);
        }
        row.trailing.removeAllViews();
        ImageButton manage = new ImageButton(activity);
        manage.setImageResource(R.drawable.ic_more_horizontal);
        manage.setColorFilter(muted);
        manage.setScaleType(ImageView.ScaleType.CENTER_INSIDE);
        manage.setPadding(dp(12), dp(12), dp(12), dp(12));
        manage.setBackground(ripple(background, 14));
        manage.setContentDescription(ui(R.string.ui_manage_pc) + store.computerName(url));
        manage.setTooltipText(ui(R.string.ui_manage_pc_label));
        manage.setOnClickListener(v -> showSavedComputerActions(url, index, count));
        row.trailing.addView(manage, new FrameLayout.LayoutParams(dp(48), dp(48), Gravity.CENTER));
    }

    private void addComputerAction(LinearLayout sheet, String label, boolean enabled, boolean danger, Runnable action) {
        Button button = new Button(activity);
        button.setText(label); button.setAllCaps(false); button.setTextSize(15);
        button.setTextColor(danger ? dangerInk : ink);
        button.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
        button.setPadding(dp(16), dp(12), dp(16), dp(12));
        button.setMinHeight(dp(48)); button.setMinimumHeight(dp(48));
        button.setStateListAnimator(null); button.setElevation(0); button.setBackgroundTintList(null);
        button.setBackground(ripple(danger ? dangerSurface : background, 14));
        button.setEnabled(enabled); button.setAlpha(enabled ? 1 : 0.4f);
        button.setOnClickListener(v -> action.run());
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.setMargins(0, dp(8), 0, 0);
        sheet.addView(button, params);
    }

    private TextView sectionTitle(String value) {
        TextView title = text(value, 15, ink);
        if (Build.VERSION.SDK_INT >= 28) title.setAccessibilityHeading(true);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        title.setPadding(0, 0, 0, dp(8));
        return title;
    }

    private LinearLayout card() {
        LinearLayout card = new LinearLayout(activity);
        card.setOrientation(LinearLayout.VERTICAL);
        card.setPadding(dp(4), dp(4), dp(4), dp(4));
        card.setBackground(rounded(surface, 21, edge));
        card.setClipToOutline(true);
        return card;
    }

    private LinearLayout.LayoutParams cardParams(int bottomMargin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.setMargins(0, 0, 0, bottomMargin);
        return params;
    }

    private View divider() {
        View line = new View(activity);
        line.setBackgroundColor(edge);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1));
        params.setMargins(dp(64), 0, dp(12), 0);
        line.setLayoutParams(params);
        return line;
    }

    private ImageView icon(int resource, int size) {
        ImageView image = new ImageView(activity);
        image.setImageResource(resource);
        image.setColorFilter(accent);
        image.setScaleType(ImageView.ScaleType.CENTER_INSIDE);
        image.setLayoutParams(new FrameLayout.LayoutParams(size, size, Gravity.CENTER));
        return image;
    }

    private static final class ComputerRow {
        final LinearLayout root;
        final TextView status;
        final LinearLayout metadata;
        final FrameLayout trailing;
        ComputerRow(LinearLayout root, TextView status, LinearLayout metadata, FrameLayout trailing) {
            this.root = root; this.status = status; this.metadata = metadata; this.trailing = trailing;
        }
    }

    private ComputerRow computerRow(String name, String address, String state, int stateColor) {
        LinearLayout row = new LinearLayout(activity);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setMinimumHeight(dp(96));
        row.setPadding(dp(12), dp(12), dp(8), dp(12));
        row.setBackground(ripple(surface, 16));
        row.setClickable(true);
        row.setFocusable(true);
        FrameLayout iconBox = new FrameLayout(activity);
        iconBox.setBackground(rounded(tile, 12, Color.TRANSPARENT));
        iconBox.addView(icon(R.drawable.ic_computer_outline, dp(24)));
        LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(dp(40), dp(40));
        iconParams.gravity = Gravity.TOP;
        row.addView(iconBox, iconParams);
        LinearLayout labels = new LinearLayout(activity);
        labels.setOrientation(LinearLayout.VERTICAL);
        LinearLayout.LayoutParams labelParams = new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        labelParams.setMargins(dp(12), 0, dp(4), 0);
        TextView title = text(name, 15, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        title.setHyphenationFrequency(android.text.Layout.HYPHENATION_FREQUENCY_NONE);
        labels.addView(title, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        TextView subtitle = text(address, 12, muted);
        subtitle.setTextDirection(View.TEXT_DIRECTION_LTR);
        subtitle.setHyphenationFrequency(android.text.Layout.HYPHENATION_FREQUENCY_NONE);
        subtitle.setPadding(0, dp(4), 0, 0);
        labels.addView(subtitle, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        LinearLayout metadata = new LinearLayout(activity) {
            @Override protected void onMeasure(int width, int height) {
                if (getChildCount() == 2 && MeasureSpec.getMode(width) != MeasureSpec.UNSPECIFIED) {
                    TextView status = (TextView) getChildAt(0);
                    TextView badge = (TextView) getChildAt(1);
                    float statusWidth = android.text.Layout.getDesiredWidth(status.getText(), status.getPaint());
                    float badgeWidth = android.text.Layout.getDesiredWidth(badge.getText(), badge.getPaint())
                        + badge.getPaddingLeft() + badge.getPaddingRight();
                    boolean inline = AdaptiveLayout.fitsInline(MeasureSpec.getSize(width), statusWidth, badgeWidth, dp(8));
                    setOrientation(inline ? HORIZONTAL : VERTICAL);
                    setGravity(inline ? Gravity.CENTER_VERTICAL : Gravity.START);
                    LinearLayout.LayoutParams badgeParams = (LinearLayout.LayoutParams) badge.getLayoutParams();
                    badgeParams.setMarginStart(inline ? dp(8) : 0);
                    badgeParams.topMargin = inline ? 0 : dp(4);
                }
                super.onMeasure(width, height);
            }
        };
        metadata.setOrientation(LinearLayout.HORIZONTAL);
        metadata.setGravity(Gravity.CENTER_VERTICAL);
        metadata.setPadding(0, dp(7), 0, 0);
        labels.addView(metadata);
        row.addView(labels, labelParams);
        TextView status = text(state, 12, stateColor);
        metadata.addView(status);
        FrameLayout trailing = new FrameLayout(activity);
        ImageView chevron = new ImageView(activity);
        chevron.setImageResource(R.drawable.ic_chevron_right);
        chevron.setColorFilter(muted);
        trailing.addView(chevron, new FrameLayout.LayoutParams(dp(20), dp(20), Gravity.CENTER));
        LinearLayout.LayoutParams trailingParams = new LinearLayout.LayoutParams(dp(48), dp(48));
        trailingParams.gravity = Gravity.TOP;
        row.addView(trailing, trailingParams);
        return new ComputerRow(row, status, metadata, trailing);
    }
}
