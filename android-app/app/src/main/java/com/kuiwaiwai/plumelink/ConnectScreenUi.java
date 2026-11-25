package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.Dialog;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Rect;
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
import android.view.inputmethod.InputMethodManager;
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
        default void onConnectRequested(String url, String identity) { onConnectRequested(url); }
        void onScanRequested();
        void onLanguageSelected(String value);
        void onRefreshScreen(String initial, String errorMessage);
        boolean isWebViewActive();
        default String connectingUrl() { return ""; }
        default int sharedFileCount() { return 0; }
        default String sharedFileMessage() { return ""; }
        default void onClearSharedFiles() {}
        default boolean canRetrySharedFiles() { return false; }
        default void onRetrySharedFiles() {}
    }

    private boolean dark;
    private int background, surface, ink, muted, availableColor, tile, edge, accent, onAccent, dangerInk, dangerSurface;

    private void updatePalette() {
        dark = AppAppearance.dark(activity);
        background = UiStyle.background(dark);
        surface = UiStyle.surface(dark);
        ink = UiStyle.ink(dark);
        muted = UiStyle.muted(dark);
        availableColor = dark ? 0xff6bd5ae : 0xff16714f;
        tile = UiStyle.accentSoft(dark);
        edge = UiStyle.edge(dark);
        accent = UiStyle.accent(dark);
        onAccent = UiStyle.onAccent(dark);
        dangerInk = UiStyle.danger(dark);
        dangerSurface = UiStyle.dangerSoft(dark);
    }
    private static final int SAVED_PC_REFRESH_MS = 4000;
    private static final int MAX_PROBE_WORKERS = 4;

    private final Activity activity;
    private final FrameLayout root;
    private final ComputerStore store;
    private final LanDiscovery discovery;
    private final Host host;

    private LinearLayout discoveryPanel;
    private LinearLayout sharePanel;
    private TextView shareTitle, shareDetail;
    private Button shareRetry;
    private final Map<String, TextView> savedStatuses = new LinkedHashMap<>();
    private final Map<String, ComputerRow> savedRows = new LinkedHashMap<>();
    private final Map<String, String> savedTargets = new LinkedHashMap<>();
    private final ExecutorService probes = Executors.newFixedThreadPool(MAX_PROBE_WORKERS);
    private final Runnable refreshSaved = this::probeSavedComputers;
    private volatile int generation;
    private int runningProbes;
    private boolean foreground;
    private boolean autoConnectLast;
    private ScrollView mainScroll;
    private Dialog activeDialog;
    private Panel activePanel = Panel.NONE;
    private String panelTarget = "";
    private int panelIndex, panelCount;
    private boolean restoringPalette;
    private enum Panel { NONE, ADDRESS, HELP, LANGUAGE, PRIVACY, COMPUTER, DETAILS, CONFIRMATION }

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
        if (activeDialog != null) activeDialog.dismiss();
        stopRefreshing();
        savedStatuses.clear();
        savedRows.clear();
        savedTargets.clear();
        discoveryPanel = null;
        sharePanel = null;
        shareTitle = shareDetail = null;
        shareRetry = null;
    }

    void destroy() {
        hide();
        probes.shutdownNow();
    }

    private String ui(int id) { return AppLanguage.text(activity, id); }
    private boolean canInteract(View source) {
        return source.isAttachedToWindow() && (activeDialog==null || activeDialog.getWindow()!=null
            && source.getRootView()==activeDialog.getWindow().getDecorView());
    }
    int dp(int value) { return Math.round(value * activity.getResources().getDisplayMetrics().density); }

    private TextView text(String value, int size, int color) {
        TextView view = new TextView(activity);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setIncludeFontPadding(false);
        view.setLetterSpacing(0);
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
        return UiStyle.ripple(activity, dark, color, radius);
    }

    private LinearLayout sheetContent() {
        LinearLayout sheet = new LinearLayout(activity);
        sheet.setOrientation(LinearLayout.VERTICAL);
        sheet.setPadding(dp(UiStyle.SHEET_GUTTER), dp(24), dp(UiStyle.SHEET_GUTTER), dp(24));
        GradientDrawable background = new GradientDrawable();
        background.setColor(surface);
        int corner = dp(UiStyle.SHEET_RADIUS);
        background.setCornerRadii(new float[]{corner, corner, corner, corner, 0, 0, 0, 0});
        sheet.setBackground(background);
        return sheet;
    }

    private void presentSheet(Dialog dialog, View content) {
        presentSheet(dialog, content, false);
    }

    private void presentSheet(Dialog dialog, View content, boolean reading) {
        autoConnectLast = false;
        activeDialog = dialog;
        FrameLayout viewport = new FrameLayout(activity) {
            private final Rect visible = new Rect();
            @Override protected void onMeasure(int widthSpec, int heightSpec) {
                int measured = MeasureSpec.getMode(heightSpec) == MeasureSpec.UNSPECIFIED ? Integer.MAX_VALUE : MeasureSpec.getSize(heightSpec);
                getWindowVisibleDisplayFrame(visible);
                int available = root.getHeight() - root.getPaddingTop() - root.getPaddingBottom();
                int limit = AdaptiveLayout.sheetLimit(measured, visible.height(), available, dp(12), reading);
                super.onMeasure(widthSpec, MeasureSpec.makeMeasureSpec(limit, reading ? MeasureSpec.EXACTLY : MeasureSpec.AT_MOST));
            }
        };
        viewport.setTag("connection-sheet");
        GradientDrawable surfaceShape = new GradientDrawable(); surfaceShape.setColor(surface);
        int corner = dp(UiStyle.SHEET_RADIUS);
        surfaceShape.setCornerRadii(new float[]{corner,corner,corner,corner,0,0,0,0});
        viewport.setBackground(surfaceShape);
        viewport.addView(content, new FrameLayout.LayoutParams(-1, reading ? -1 : -2));
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE);
        dialog.setContentView(viewport);
        dialog.setCanceledOnTouchOutside(true);
        Window window = dialog.getWindow();
        if (window != null) {
            window.setBackgroundDrawableResource(android.R.color.transparent);
            window.setGravity(Gravity.BOTTOM);
            window.addFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND);
            window.setDimAmount(0.40f);
            window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        }
        dialog.show();
        if (window != null) {
            Runnable resize = () -> {
                int width = root.getWidth() - root.getPaddingLeft() - root.getPaddingRight();
                window.setLayout(width > 0 ? Math.min(width, dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP)) : -1, -2);
                viewport.requestLayout();
            };
            resize.run();
            View.OnLayoutChangeListener listener = (v, l, t, r, b, oldL, oldT, oldR, oldB) -> resize.run();
            root.addOnLayoutChangeListener(listener);
            dialog.setOnDismissListener(ignored -> {
                root.removeOnLayoutChangeListener(listener);
                if (activeDialog == dialog) { activeDialog=null; activePanel=Panel.NONE; }
            });
        }
    }

    private void beginPanel(Panel panel, String target, int index, int count) {
        autoConnectLast=false;
        if (activeDialog != null) activeDialog.dismiss();
        activePanel=panel;panelTarget=target;panelIndex=index;panelCount=count;
    }

    AlertDialog showSecurityDialog(AlertDialog.Builder builder) {
        updatePalette();
        beginPanel(Panel.CONFIRMATION, "", 0, 0);
        AlertDialog dialog = builder.create();
        activeDialog=dialog;
        dialog.setOnShowListener(ignored -> tintConfirmation(dialog));
        dialog.setOnDismissListener(ignored -> { if (activeDialog==dialog) {activeDialog=null;activePanel=Panel.NONE;} });
        dialog.show();
        return dialog;
    }
    private void tintConfirmation(AlertDialog dialog) {
        if (dialog.getWindow() == null) return;
        dialog.getWindow().setBackgroundDrawable(rounded(surface,24,Color.TRANSPARENT));
        tintConfirmationText(dialog.getWindow().getDecorView());
        for (int which : new int[]{AlertDialog.BUTTON_POSITIVE,AlertDialog.BUTTON_NEGATIVE,AlertDialog.BUTTON_NEUTRAL}) {
            Button button=dialog.getButton(which);
            if (button!=null) {UiStyle.button(button);button.setTextColor(which==AlertDialog.BUTTON_NEGATIVE?muted:accent);button.setBackground(ripple(Color.TRANSPARENT,UiStyle.BUTTON_RADIUS));}
        }
    }
    private void tintConfirmationText(View view) {
        if (view instanceof TextView) ((TextView)view).setTextColor(ink);
        if (view instanceof ViewGroup) for(int i=0;i<((ViewGroup)view).getChildCount();i++) tintConfirmationText(((ViewGroup)view).getChildAt(i));
    }

    @SuppressLint("SetTextI18n")
    private String shownAddress = "", shownError = "";
    boolean themeChanged() { return dark != AppAppearance.dark(activity); }
    void refreshPalette() {
        int scroll=mainScroll==null?0:mainScroll.getScrollY();
        Dialog retained=activeDialog instanceof AlertDialog?activeDialog:null;
        PanelSnapshot snapshot=retained==null?capturePanel():null;
        if(retained!=null) activeDialog=null;
        restoringPalette=true;
        try {show(shownAddress,shownError);} finally {restoringPalette=false;}
        if(mainScroll!=null) restoreScroll(mainScroll,scroll);
        if(retained!=null) {activeDialog=retained;activePanel=Panel.CONFIRMATION;tintConfirmation((AlertDialog)retained);}
        else if(snapshot!=null) restorePanel(snapshot);
    }
    private static final class Draft {
        final String text,error;final int start,end;final boolean focused;
        Draft(EditText field) {text=field.getText().toString();error=field.getError()==null?null:field.getError().toString();start=field.getSelectionStart();end=field.getSelectionEnd();focused=field.hasFocus();}
    }
    private static final class PanelSnapshot {
        Panel panel;String target;int index,count,scroll;boolean ime;
        final Map<String,Draft> drafts=new LinkedHashMap<>();
    }
    private PanelSnapshot capturePanel() {
        if(activeDialog==null || activeDialog.getWindow()==null || activePanel==Panel.NONE) return null;
        PanelSnapshot snapshot=new PanelSnapshot();snapshot.panel=activePanel;snapshot.target=panelTarget;snapshot.index=panelIndex;snapshot.count=panelCount;
        View decor=activeDialog.getWindow().getDecorView();
        for(String tag:new String[]{"connect-address","computer-alias","computer-location"}) {View field=decor.findViewWithTag(tag);if(field instanceof EditText) snapshot.drafts.put(tag,new Draft((EditText)field));}
        View scroll=decor.findViewWithTag("connection-sheet-scroll");if(scroll!=null)snapshot.scroll=scroll.getScrollY();
        if(Build.VERSION.SDK_INT>=30 && decor.getRootWindowInsets()!=null) snapshot.ime=decor.getRootWindowInsets().isVisible(android.view.WindowInsets.Type.ime());
        else {Rect visible=new Rect();decor.getWindowVisibleDisplayFrame(visible);snapshot.ime=visible.height()>0&&activity.getResources().getDisplayMetrics().heightPixels-visible.height()>dp(120);}
        return snapshot;
    }
    private void restorePanel(PanelSnapshot snapshot) {
        switch(snapshot.panel) {
            case ADDRESS -> showManualConnectSheet(snapshot.target);
            case HELP -> showConnectionHelp();
            case LANGUAGE -> showLanguagePicker();
            case PRIVACY -> showPrivacyPolicy();
            case COMPUTER -> showSavedComputerActions(snapshot.target,snapshot.index,snapshot.count);
            case DETAILS -> showComputerDetailsEditor(snapshot.target);
            default -> {return;}
        }
        if(activeDialog==null || activeDialog.getWindow()==null) return;
        View decor=activeDialog.getWindow().getDecorView();
        for(Map.Entry<String,Draft> entry:snapshot.drafts.entrySet()) {
            EditText field=decor.findViewWithTag(entry.getKey());if(field==null)continue;
            Draft draft=entry.getValue();field.setText(draft.text);
            if(draft.error!=null)field.setError(draft.error);
            field.setSelection(Math.max(0,Math.min(field.length(),draft.start)),Math.max(0,Math.min(field.length(),draft.end)));
            if(draft.focused) {field.requestFocus();if(snapshot.ime)restoreKeyboard(field,activeDialog);}
        }
        View scroll=decor.findViewWithTag("connection-sheet-scroll");if(scroll!=null)restoreScroll(scroll,snapshot.scroll);
    }
    private void restoreScroll(View scroll,int position) {
        scroll.getViewTreeObserver().addOnPreDrawListener(new android.view.ViewTreeObserver.OnPreDrawListener() {
            @Override public boolean onPreDraw() {
                scroll.getViewTreeObserver().removeOnPreDrawListener(this);scroll.scrollTo(0,position);return true;
            }
        });
    }
    private void restoreKeyboard(EditText field,Dialog owner) {
        Runnable show=()->{if(activeDialog==owner&&field.isAttachedToWindow())((InputMethodManager)activity.getSystemService(Activity.INPUT_METHOD_SERVICE)).showSoftInput(field,InputMethodManager.SHOW_IMPLICIT);};
        if(field.hasWindowFocus())field.post(show);
        else field.getViewTreeObserver().addOnWindowFocusChangeListener(new android.view.ViewTreeObserver.OnWindowFocusChangeListener() {
            @Override public void onWindowFocusChanged(boolean focused) {if(focused){field.getViewTreeObserver().removeOnWindowFocusChangeListener(this);field.post(show);}}
        });
    }

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
        sharePanel = new LinearLayout(activity);
        sharePanel.setOrientation(LinearLayout.VERTICAL);
        sharePanel.setPadding(dp(14), dp(8), dp(14), dp(8));
        sharePanel.setBackground(rounded(tile, 16, Color.TRANSPARENT));
        panel.addView(sharePanel, cardParams(dp(12)));
        renderSharedFiles();
        if (!errorMessage.isEmpty()) {
            TextView error = text(errorMessage, 13, dangerInk);
            error.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
            error.setPadding(dp(14), dp(11), dp(14), dp(11));
            error.setBackground(rounded(dangerSurface, 16, edge));
            LinearLayout.LayoutParams errorParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            errorParams.setMargins(0, 0, 0, dp(14));
            panel.addView(error, errorParams);
            if (!restoringPalette) error.post(() -> error.requestRectangleOnScreen(
                new android.graphics.Rect(0, 0, error.getWidth(), error.getHeight()), true));
        }

        ArrayList<String> savedComputers = store.visibleSavedRecords();
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
                savedRow.root.setOnClickListener(v -> host.onConnectRequested(store.endpointFor(savedUrl),store.storedIdentityOrEmpty(savedUrl)));
                savedRow.root.setOnLongClickListener(v -> {
                    showSavedComputerActions(savedUrl, groupIndex, groupCount);
                    return true;
                });
                if (index > 0) savedCard.addView(divider());
                savedCard.addView(savedRow.root);
                savedStatuses.put(savedUrl, savedRow.status);
                savedRows.put(savedUrl, savedRow);
                savedTargets.put(savedUrl, store.endpointFor(savedUrl));
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
        refresh.setPadding(dp(12), 0, dp(12), 0);
        refresh.setBackground(ripple(Color.TRANSPARENT, UiStyle.BUTTON_RADIUS));
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
        mainScroll = scroll;
        refreshSavedComputers();

        LinearLayout dock = new LinearLayout(activity);
        dock.setOrientation(LinearLayout.VERTICAL);
        dock.setPadding(dp(20), dp(16), dp(20), dp(16));
        dock.setBackground(rounded(surface, 0, Color.TRANSPARENT));
        dock.setElevation(dp(8));
        Button manual = new Button(activity);
        UiStyle.button(manual);
        manual.setAllCaps(false);
        manual.setText(ui(R.string.ui_scan_to_connect));
        manual.setTextColor(onAccent);
        manual.setTextSize(14);
        manual.setMinHeight(dp(54));
        manual.setMinimumHeight(dp(54));manual.setStateListAnimator(null);manual.setElevation(0);
        manual.setPadding(0, 0, 0, 0);
        manual.setBackground(ripple(accent, 16));
        manual.setOnClickListener(v -> {if(canInteract(v)){autoConnectLast=false;host.onScanRequested();}});
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
        ConnectWorkspace screen = new ConnectWorkspace(activity,panel,scroll,dock,dockMeta,logo,title,subtitle,surface);
        root.addView(screen,new FrameLayout.LayoutParams(-1,-1));
    }

    private Button dockAction(int label, int color, Runnable action) {
        Button button = new Button(activity);
        UiStyle.button(button);
        button.setText(ui(label));
        button.setTextColor(color);
        button.setTextSize(13);
        button.setAllCaps(false);
        button.setMinimumHeight(dp(48));
        button.setPadding(dp(8), dp(6), dp(8), dp(6));
        button.setStateListAnimator(null);
        button.setBackground(ripple(Color.TRANSPARENT, 14));
        button.setOnClickListener(v -> {if(canInteract(v))action.run();});
        return button;
    }

    void renderSharedFiles() {
        if (sharePanel == null) return;
        int count = host.sharedFileCount();
        String message = host.sharedFileMessage();
        sharePanel.setVisibility(count == 0 && message.isEmpty() ? View.GONE : View.VISIBLE);
        if (count == 0 && message.isEmpty()) return;
        if (shareTitle == null) {
            LinearLayout top = new LinearLayout(activity);
            top.setGravity(Gravity.CENTER_VERTICAL);
            shareTitle = text("", 15, ink);
            shareTitle.setTypeface(null, android.graphics.Typeface.BOLD);
            top.addView(shareTitle, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
            top.addView(dockAction(R.string.ui_share_clear, accent, host::onClearSharedFiles));
            sharePanel.addView(top);
            shareDetail = text("", 13, muted);
            shareDetail.setPadding(0, 0, 0, dp(6));
            sharePanel.addView(shareDetail);
            shareRetry = dockAction(R.string.ui_share_retry, accent, host::onRetrySharedFiles);
            shareRetry.setTag("share-retry");
            sharePanel.addView(shareRetry);
        }
        shareTitle.setText(String.format(java.util.Locale.ROOT, ui(R.string.ui_share_pending), count));
        shareDetail.setText(message.isEmpty() ? ui(R.string.ui_share_connect_first) : message);
        shareRetry.setVisibility(host.canRetrySharedFiles() ? View.VISIBLE : View.GONE);
    }

    void renderDiscoveredComputers() {
        if (discoveryPanel == null) return;
        updateSavedTargets();
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
                    known.isEmpty() ? ui(R.string.ui_nearby_windows_pc) : store.computerName(identity),
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

    void renderConnecting() {
        for (Map.Entry<String, TextView> entry : savedStatuses.entrySet()) {
            String target = savedTargets.getOrDefault(entry.getKey(), entry.getKey());
            TextView label = entry.getValue();
            if (target.equals(host.connectingUrl())) { label.setText(ui(R.string.ui_verifying_pc)); label.setTextColor(muted); }
            else if (ui(R.string.ui_verifying_pc).contentEquals(label.getText())) label.setText(ui(R.string.ui_checking));
        }
    }

    private void updateSavedTargets() {
        boolean changed = false;
        for (Map.Entry<String, ComputerRow> entry : savedRows.entrySet()) {
            String saved = entry.getKey();
            String endpoint = store.endpointFor(saved);
            String target = EndpointUtils.isUsbEndpoint(endpoint) ? endpoint : DiscoveredTargets.choose(endpoint, store.storedIdentityOrEmpty(saved), discovery.computers(), discovery::identityFor);
            if (target.equals(savedTargets.get(saved))) continue;
            savedTargets.put(saved, target);
            ComputerRow row = entry.getValue();
            row.subtitle.setText(target.equals(endpoint) ? store.computerSubtitle(saved) : target.replace("https://", ""));
            row.status.setText(ui(target.equals(endpoint) ? R.string.ui_checking : R.string.ui_address_updated));
            row.status.setTextColor(muted);
            row.root.setOnClickListener(v -> host.onConnectRequested(savedTargets.getOrDefault(saved, endpoint),store.storedIdentityOrEmpty(saved)));
            changed = true;
        }
        if (changed) { stopRefreshing(); refreshSavedComputers(); }
    }

    private void probeSavedComputers() {
        if (!foreground || host.isWebViewActive() || savedStatuses.isEmpty() || runningProbes != 0) return;
        int targetGen = generation;
        ArrayList<String> urls = new ArrayList<>(savedStatuses.keySet());
        Map<String, String> targets = new LinkedHashMap<>(savedTargets);
        int workers = Math.min(MAX_PROBE_WORKERS, urls.size());
        runningProbes = workers;
        for (int worker = 0; worker < workers; worker++) {
            int start = worker;
            probes.execute(() -> {
                try {
                    for (int i = start; i < urls.size() && targetGen == generation; i += workers) {
                        String url = urls.get(i);
                        String target = targets.getOrDefault(url, url);
                        int status = savedServiceStatus(target, store.storedIdentityOrEmpty(url));
                        activity.runOnUiThread(() -> applySavedStatus(targetGen, url, target, status));
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

    private int savedServiceStatus(String url, String identity) {
        if (!EndpointUtils.isValidPrivateEndpoint(url)) return R.string.ui_unreachable;
        String hostName = android.net.Uri.parse(url).getHost();
        String pin = store.trustedPin(hostName, identity);
        if (!pin.isEmpty()) {
            javax.net.ssl.HttpsURLConnection connection = null;
            try {
                // The paired service requires authentication on this route. Never send a saved token in a background probe.
                connection = new PinnedConnections(hostName, pin, url, store.usbAccess(url)).open(url + "/api/status", "");
                connection.setConnectTimeout(1400);
                connection.setReadTimeout(1400);
                if (connection.getResponseCode() == 401) return R.string.ui_available;
                if (EndpointUtils.isUsbEndpoint(url) && connection.getResponseCode() == 410) return R.string.ui_usb_connection_expired_status;
            } catch (Exception ignored) {
                // A listening address can still be starting up, or its certificate may have changed.
            } finally {
                if (connection != null) connection.disconnect();
            }
        }
        return EndpointUtils.isReachable(url) ? R.string.ui_service_unconfirmed : R.string.ui_unreachable;
    }

    private void applySavedStatus(int targetGen, String url, String target, int result) {
        if (targetGen != generation || !foreground || host.isWebViewActive()) return;
        if (!target.equals(savedTargets.get(url))) return;
        if (target.equals(host.connectingUrl())) return;
        TextView status = savedStatuses.get(url);
        if (status == null) return;
        boolean available = result == R.string.ui_available;
        String label = ui(result);
        if (!target.equals(store.endpointFor(url))) label = ui(R.string.ui_address_updated) + " · " + label;
        if (!label.contentEquals(status.getText())) {
            status.setText(label);
            status.setTextColor(available ? availableColor : muted);
        }
        if (autoConnectLast && (url.equals(store.lastIdentity()) || url.equals(store.lastUrl()))) {
            autoConnectLast = false;
            if (available && target.equals(store.endpointFor(url))) host.onConnectRequested(target,store.storedIdentityOrEmpty(url));
        }
    }

    private void showManualConnectSheet(String initial) {
        beginPanel(Panel.ADDRESS,initial,0,0);
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView title = text(ui(R.string.ui_enter_pc_address), 20, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(title);
        TextView description = text(ui(R.string.ui_find_the_address_below_the_qr_code_on_the_windows_pc), 14, muted);
        description.setPadding(0, dp(8), 0, dp(22));
        sheet.addView(description);
        EditText address = new EditText(activity);
        address.setTag("connect-address");
        address.setId(View.generateViewId());title.setLabelFor(address.getId());
        address.setHint(String.format(java.util.Locale.ROOT, ui(R.string.ui_pc_address_example), EndpointUtils.DEFAULT_SERVICE_PORT));
        address.setSingleLine(true);
        address.setTextColor(ink);
        address.setHintTextColor(muted);
        address.setTextSize(16);
        UiStyle.field(address, dark);
        address.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_VARIATION_URI);
        address.setImeOptions(EditorInfo.IME_ACTION_GO);
        if (!initial.isEmpty()) address.setText(initial.replace("https://", ""));
        address.setMinimumHeight(dp(52));
        sheet.addView(address, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        Button submit = new Button(activity);
        UiStyle.button(submit);
        submit.setAllCaps(false);
        submit.setText(ui(R.string.ui_connect_to_pc));
        submit.setTextSize(14);
        submit.setTextColor(onAccent);
        submit.setMinHeight(dp(52));
        submit.setBackground(ripple(accent, UiStyle.BUTTON_RADIUS));
        LinearLayout.LayoutParams submitParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        submitParams.setMargins(0, dp(16), 0, 0);
        submit.setOnClickListener(v -> {
            if(!canInteract(v))return;
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
        scroll.setTag("connection-sheet-scroll");
        scroll.addView(sheet);
        presentSheet(dialog, scroll);
    }

    private void showConnectionHelp() {
        beginPanel(Panel.HELP,"",0,0);
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView title = text(ui(R.string.ui_connection_help), 20, ink);
        title.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(title);
        TextView instructions = text(ui(R.string.ui_1_start_remote_control_on_the_windows_pc_2_join_the_sam), 14, muted);
        instructions.setLineSpacing(dp(5), 1f);
        instructions.setPadding(0, dp(17), 0, dp(14));
        sheet.addView(instructions);
        TextView usbHelp = text(ui(R.string.ui_usb_help), 14, muted);
        usbHelp.setLineSpacing(dp(5), 1f); usbHelp.setPadding(0,0,0,dp(14)); sheet.addView(usbHelp);
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
            UiStyle.button(notifyLink);
            notifyLink.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
            notifyLink.setBackground(ripple(UiStyle.tonal(dark), UiStyle.BUTTON_RADIUS));
            notifyLink.setOnClickListener(v -> {
                dialog.dismiss();
                try {
                    Intent settingsIntent = new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, activity.getPackageName());
                    activity.startActivity(settingsIntent);
                } catch (Exception ignored) {}
            });
            LinearLayout.LayoutParams notifyParams = new LinearLayout.LayoutParams(-1, -2);
            notifyParams.topMargin = dp(8);
            sheet.addView(notifyLink, notifyParams);
        }
        if (store.hasTrustRecord()) {
            TextView reset = text(ui(R.string.ui_reset_trusted_certificates), 14, accent);
            reset.setTypeface(null, android.graphics.Typeface.BOLD);
            reset.setGravity(Gravity.CENTER_VERTICAL);
            UiStyle.button(reset);
            reset.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
            reset.setTextColor(dangerInk);
            reset.setBackground(ripple(dangerSurface, UiStyle.BUTTON_RADIUS));
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
            LinearLayout.LayoutParams resetParams = new LinearLayout.LayoutParams(-1, -2);
            resetParams.topMargin = dp(8);
            sheet.addView(reset, resetParams);
        }
        ScrollView scroll = new ScrollView(activity);
        scroll.setTag("connection-sheet-scroll");
        scroll.addView(sheet);
        presentSheet(dialog, scroll);
    }

    private void showLanguagePicker() {
        beginPanel(Panel.LANGUAGE,"",0,0);
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
            option.setTextSize(14);
            option.setIncludeFontPadding(false);
            option.setButtonDrawable((android.graphics.drawable.Drawable) null);
            option.setTextColor(selected ? accent : ink);
            option.setButtonTintList(ColorStateList.valueOf(selected ? accent : muted));
            option.setBackground(ripple(selected ? tile : UiStyle.tonal(dark), UiStyle.BUTTON_RADIUS));
            GradientDrawable marker = rounded(selected ? accent : Color.TRANSPARENT, 10, selected ? Color.TRANSPARENT : muted);
            marker.setBounds(0, 0, dp(20), dp(20));
            if (selected) {
                android.graphics.drawable.LayerDrawable checked = new android.graphics.drawable.LayerDrawable(new android.graphics.drawable.Drawable[]{marker, NativeIcon.of("check", onAccent, dp(20))});
                checked.setBounds(0, 0, dp(20), dp(20));
                option.setCompoundDrawablesRelative(null, null, checked, null);
            } else option.setCompoundDrawablesRelative(null, null, marker, null);
            option.setCompoundDrawablePadding(dp(12));
            option.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
            option.setPadding(dp(14), dp(12), dp(14), dp(12));
            option.setMinimumHeight(dp(52));
            RadioGroup.LayoutParams params = new RadioGroup.LayoutParams(-1, -2);
            params.topMargin = i == 0 ? 0 : dp(6);
            choices.addView(option, params);
            if (selected) choices.check(option.getId());
            option.setOnClickListener(v -> {
                if(!canInteract(v))return;
                dialog.dismiss();
                host.onLanguageSelected(value);
                host.onRefreshScreen("", "");
            });
        }
        presentInfoSheet(dialog, R.string.ui_language, choices, false);
    }

    void showPrivacyPolicy() {
        beginPanel(Panel.PRIVACY,"",0,0);
        updatePalette();
        TextView body = text("", 15, ink);
        body.setText(PrivacyPolicy.content(activity));
        body.setLineSpacing(dp(4), 1f);
        body.setTextIsSelectable(true);
        presentInfoSheet(new Dialog(activity), R.string.ui_privacy_policy, body, true);
    }

    private void presentInfoSheet(Dialog dialog, int titleId, View content, boolean reading) {
        LinearLayout sheet = sheetContent();
        sheet.setPadding(dp(UiStyle.SHEET_GUTTER), dp(12), dp(UiStyle.SHEET_GUTTER), dp(24));
        LinearLayout header = new LinearLayout(activity);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text(ui(titleId), 20, ink);
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
        scroll.setTag("connection-sheet-scroll");
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
        beginPanel(Panel.COMPUTER,url,index,count);
        Dialog dialog = new Dialog(activity);
        LinearLayout sheet = sheetContent();
        TextView heading = text(store.computerName(url), 20, ink);
        heading.setTypeface(null, android.graphics.Typeface.BOLD);
        sheet.addView(heading);
        TextView address = text(store.computerSubtitle(url), 13, muted);
        address.setTextDirection(View.TEXT_DIRECTION_LTR);
        address.setPadding(0, dp(6), 0, dp(12));
        sheet.addView(address);
        LinearLayout commands = new LinearLayout(activity);
        commands.setOrientation(LinearLayout.VERTICAL);
        String wirelessEndpoint = store.wirelessEndpoint(url), usbEndpoint = store.usbEndpoint(url);
        if (!wirelessEndpoint.isEmpty() && !usbEndpoint.isEmpty()) {
            addComputerAction(commands, ui(R.string.ui_connect_wirelessly), true, false, () -> {
                dialog.dismiss(); store.preferUsb(url,false); host.onConnectRequested(wirelessEndpoint,store.storedIdentityOrEmpty(url));
            });
            addComputerAction(commands, ui(R.string.ui_usb_connection), true, false, () -> {
                dialog.dismiss(); store.preferUsb(url,true); host.onConnectRequested(usbEndpoint,store.storedIdentityOrEmpty(url));
            });
        }
        addComputerAction(commands, ui(R.string.ui_edit_name_and_location), true, false, () -> { dialog.dismiss(); showComputerDetailsEditor(url); });
        boolean pinned = store.isPinned(url);
        addComputerAction(commands, pinned ? ui(R.string.ui_unpin) : ui(R.string.ui_pin_pc), true, false, () -> {
            dialog.dismiss(); store.togglePinned(url); host.onRefreshScreen("", "");
        });
        addComputerAction(commands, ui(R.string.ui_move_up), index > 0, false, () -> {
            dialog.dismiss(); store.moveSavedComputer(url, -1); host.onRefreshScreen("", "");
        });
        addComputerAction(commands, ui(R.string.ui_move_down), index + 1 < count, false, () -> {
            dialog.dismiss(); store.moveSavedComputer(url, 1); host.onRefreshScreen("", "");
        });
        String identity = store.identityForUrl(url);
        if (!store.token(identity).isEmpty()) {
            addComputerAction(commands, ui(R.string.ui_pair_again), true, false, () -> {
                dialog.dismiss();
                showSecurityDialog(AppAppearance.dialog(activity).setTitle(ui(R.string.ui_pair_again))
                    .setMessage(store.computerName(url) + "\n\n" + ui(R.string.ui_pair_again_explanation))
                    .setNegativeButton(ui(R.string.ui_cancel), null)
                    .setPositiveButton(ui(R.string.ui_pair_again), (ignored, which) -> {
                        store.setToken(identity, "");
                        host.onConnectRequested(store.endpointFor(url),store.storedIdentityOrEmpty(url));
                    }));
            });
        }
        addComputerAction(commands, ui(R.string.ui_remove_this_pc), true, true, () -> {
            dialog.dismiss();
            showSecurityDialog(AppAppearance.dialog(activity).setTitle(ui(R.string.ui_remove_this_pc_label))
                .setMessage(store.computerName(url) + ui(R.string.ui_will_be_removed_from_saved_pcs_along_with_its_pairing_c))
                .setNegativeButton(ui(R.string.ui_cancel), null)
                .setPositiveButton(ui(R.string.ui_remove), (ignored, which) -> {
                    store.forgetComputer(url);
                    host.onRefreshScreen("", ui(R.string.ui_pc_removed));
                }));
        });
        addComputerAction(commands, ui(R.string.ui_cancel), true, false, dialog::dismiss);
        ScrollView scroll = new ScrollView(activity);
        scroll.setTag("connection-sheet-scroll");
        scroll.setFillViewport(false);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.addView(commands);
        sheet.addView(scroll,new LinearLayout.LayoutParams(-1,-2));
        presentSheet(dialog, sheet);
    }

    private void showComputerDetailsEditor(String url) {
        beginPanel(Panel.DETAILS,url,0,0);
        Dialog dialog=new Dialog(activity);
        LinearLayout form=sheetContent();
        TextView title=text(ui(R.string.ui_pc_name_and_location),20,ink);title.setTypeface(null,android.graphics.Typeface.BOLD);form.addView(title);
        EditText alias=detailsField(form,R.string.ui_pc_name,R.string.ui_name_e_g_study_pc,"computer-alias",store.computerMetadata(url,ComputerStore.ALIAS_PREFIX),40);
        EditText location=detailsField(form,R.string.ui_pc_location,R.string.ui_location_e_g_upstairs_study,"computer-location",store.computerMetadata(url,ComputerStore.LOCATION_PREFIX),60);
        alias.setImeOptions(EditorInfo.IME_ACTION_NEXT);location.setImeOptions(EditorInfo.IME_ACTION_DONE);
        Button save=dockAction(R.string.ui_save,onAccent,()->{
            dialog.dismiss();store.saveComputerMetadata(url,alias.getText().toString(),location.getText().toString());host.onRefreshScreen("",ui(R.string.ui_pc_details_saved));
        });
        save.setBackground(ripple(accent,UiStyle.BUTTON_RADIUS));
        LinearLayout.LayoutParams saveParams=new LinearLayout.LayoutParams(-1,-2);saveParams.topMargin=dp(20);form.addView(save,saveParams);
        LinearLayout actions=new LinearLayout(activity);
        actions.addView(dockAction(R.string.ui_clear,muted,()->{dialog.dismiss();store.saveComputerMetadata(url,"","");host.onRefreshScreen("",ui(R.string.ui_pc_details_cleared));}),new LinearLayout.LayoutParams(0,-2,1));
        actions.addView(dockAction(R.string.ui_cancel,muted,dialog::dismiss),new LinearLayout.LayoutParams(0,-2,1));
        LinearLayout.LayoutParams actionParams=new LinearLayout.LayoutParams(-1,-2);actionParams.topMargin=dp(8);form.addView(actions,actionParams);
        ScrollView formScroll = new ScrollView(activity);
        formScroll.setTag("connection-sheet-scroll");
        formScroll.addView(form);
        presentSheet(dialog,formScroll);
    }
    private EditText detailsField(LinearLayout parent,int labelId,int hintId,String tag,String value,int limit) {
        TextView label=text(ui(labelId),13,muted);label.setPadding(0,dp(16),0,dp(8));parent.addView(label);
        EditText field=new EditText(activity);field.setId(View.generateViewId());field.setTag(tag);label.setLabelFor(field.getId());
        field.setTextColor(ink);field.setHintTextColor(muted);field.setTextSize(16);field.setSingleLine(true);
        field.setHint(ui(hintId));field.setText(value);field.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(limit)});
        UiStyle.field(field,dark);
        parent.addView(field,new LinearLayout.LayoutParams(-1,-2));return field;
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
        manage.setImageDrawable(NativeIcon.of("more", muted, dp(UiStyle.ICON)));
        manage.setScaleType(ImageView.ScaleType.CENTER_INSIDE);
        manage.setPadding(dp(14), dp(14), dp(14), dp(14));
        manage.setBackground(ripple(background, 14));
        manage.setContentDescription(ui(R.string.ui_manage_pc) + store.computerName(url));
        manage.setTooltipText(ui(R.string.ui_manage_pc_label));
        manage.setOnClickListener(v -> showSavedComputerActions(url, index, count));
        row.trailing.addView(manage, new FrameLayout.LayoutParams(dp(48), dp(48), Gravity.CENTER));
    }

    private void addComputerAction(LinearLayout sheet, String label, boolean enabled, boolean danger, Runnable action) {
        Button button = new Button(activity);
        UiStyle.button(button);
        button.setText(label); button.setAllCaps(false); button.setTextSize(14);
        button.setTextColor(danger ? dangerInk : ink);
        button.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
        button.setPadding(dp(16), dp(12), dp(16), dp(12));
        button.setMinHeight(dp(56)); button.setMinimumHeight(dp(56));
        button.setStateListAnimator(null); button.setElevation(0); button.setBackgroundTintList(null);
        button.setBackground(ripple(danger ? dangerSurface : UiStyle.tonal(dark), UiStyle.BUTTON_RADIUS));
        button.setEnabled(enabled); button.setAlpha(enabled ? 1 : .44f);
        button.setOnClickListener(v -> {if(canInteract(v))action.run();});
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
        final TextView subtitle;
        final LinearLayout metadata;
        final FrameLayout trailing;
        ComputerRow(LinearLayout root, TextView status, TextView subtitle, LinearLayout metadata, FrameLayout trailing) {
            this.root = root; this.status = status; this.subtitle = subtitle; this.metadata = metadata; this.trailing = trailing;
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
        return new ComputerRow(row, status, subtitle, metadata, trailing);
    }
}
