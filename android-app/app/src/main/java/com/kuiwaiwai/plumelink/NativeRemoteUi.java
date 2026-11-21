package com.kuiwaiwai.plumelink;

import android.app.Activity;
import android.app.Dialog;
import android.content.res.ColorStateList;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.InsetDrawable;
import android.graphics.drawable.LayerDrawable;
import android.graphics.drawable.RippleDrawable;
import android.os.Build;
import android.text.Editable;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.InputFilter;
import android.text.InputType;
import android.text.TextWatcher;
import android.text.TextUtils;
import android.text.Layout;
import android.text.method.LinkMovementMethod;
import android.text.style.URLSpan;
import android.text.StaticLayout;
import android.text.TextPaint;
import android.text.style.ForegroundColorSpan;
import android.text.style.RelativeSizeSpan;
import android.util.Base64;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.util.TypedValue;
import java.text.BreakIterator;
import java.util.Locale;
import android.widget.*;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/** Android views for the remote workspace. The existing session delivers state and receives actions. */
final class NativeRemoteUi {
    private static final int SHEET_GUTTER_DP = 20;
    private static final int MORE_GAP_DP = 8;
    private static final int MORE_TILE_PADDING_DP = 6;
    private static final int MORE_TILE_TEXT_SP = 12;
    private static final int STANDARD_DIAL_DP = 208;
    private static final int DIAL_MIN_HEIGHT_DP = 156;
    interface Host {
        void action(String id, Object value);
        void pointer(JSONObject sample);
    }

    private final Activity activity;
    private final FrameLayout root;
    private final FrameLayout pageFrame;
    private final Host host;
    private final LinearLayout screen, page, content, navigation;
    private final WorkspaceHeader header;
    private final LinearLayout headerActions;
    private final ImageButton headerFocus;
    private final FrameLayout headerBack;
    private final ImageView headerBackIcon;
    private final ScrollView scroll;
    private final ScrollView navigationRail;
    private final TextView title, subtitle, feedback;
    private Dialog dialog;
    private PopupWindow permissionHint;
    private SheetLayout sheetLayout;
    private LinearLayout sheetContent;
    private ScrollView sheetScroll;
    private TextView sheetTitle;
    private View handle;
    private boolean compactMoreSheet;
    private String sheetKey = "";
    private JSONObject latest;
    private JSONObject microphoneState = new JSONObject();
    private final NativeMicrophoneUi microphoneUi;
    private final NativeMicrophoneUi.HeaderStatus microphoneStatus;
    private int viewportHeight;
    private boolean shortKeyboard;
    private NativeMicrophoneUi.Capsule microphoneCapsule;
    private final java.util.WeakHashMap<View, String> appliedState = new java.util.WeakHashMap<>();
    private boolean dark, wide, tabletWide, compact, editing;
    private boolean filesTwoPane;
    private int pageGutter = 16;

    NativeRemoteUi(Activity activity, FrameLayout root, Host host) {
        this.activity = activity;
        this.root = root;
        this.host = host;
        microphoneUi = new NativeMicrophoneUi(activity);
        screen = column();
        page = new LinearLayout(activity) {
            @Override protected void onMeasure(int width, int height) {
                boolean shortEntry = "keyboard".equals(content.getTag()) && MeasureSpec.getSize(width) >= dp(480)
                    && MeasureSpec.getSize(height) < dp(200);
                if (shortKeyboard != shortEntry) configureKeyboardViewport(shortEntry);
                super.onMeasure(width, height);
            }
        };
        page.setOrientation(LinearLayout.VERTICAL);
        pageFrame = new FrameLayout(activity);
        pageFrame.addView(page, new FrameLayout.LayoutParams(-1, -1));
        screen.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        header = new WorkspaceHeader();
        header.setGravity(Gravity.CENTER_VERTICAL);
        headerBack = new FrameLayout(activity);
        headerBack.setBackground(ripple(Color.TRANSPARENT, 24));
        headerBack.setFocusable(true);
        headerBack.setContentDescription(AppLanguage.text(activity, R.string.ui_back_to_remote));
        headerBack.setOnClickListener(v -> emit("back-remote", null));
        headerBackIcon = new ImageView(activity);
        headerBack.addView(headerBackIcon, new FrameLayout.LayoutParams(dp(24), dp(24), Gravity.CENTER));
        header.addView(headerBack, 0, new LinearLayout.LayoutParams(dp(48), dp(48)));
        headerBack.setVisibility(View.GONE);
        LinearLayout identity = column();
        title = label(26);
        title.setTypeface(null, Typeface.BOLD);
        markHeading(title);
        title.setSingleLine();
        title.setEllipsize(TextUtils.TruncateAt.END);
        subtitle = label(13);
        subtitle.setSingleLine();
        subtitle.setEllipsize(android.text.TextUtils.TruncateAt.END);
        identity.addView(title);
        FrameLayout subtitleSlot = new FrameLayout(activity);
        subtitleSlot.setMeasureAllChildren(true);
        subtitleSlot.addView(subtitle);
        microphoneStatus = microphoneUi.new HeaderStatus(() -> emit("mic-stop", null));
        subtitleSlot.addView(microphoneStatus, new FrameLayout.LayoutParams(-1, -1));
        microphoneStatus.setVisibility(View.GONE);
        identity.addView(subtitleSlot);
        LinearLayout identityRow = new LinearLayout(activity);
        identityRow.setGravity(Gravity.CENTER_VERTICAL);
        identityRow.setMinimumHeight(dp(48));
        identityRow.addView(identity, new LinearLayout.LayoutParams(0, -2, 1));
        identityRow.addView(microphoneStatus.stopButton(), new LinearLayout.LayoutParams(dp(48), dp(48)));
        microphoneStatus.stopButton().setVisibility(View.GONE);
        header.addView(identityRow, new LinearLayout.LayoutParams(0, -2, 1));
        headerActions = new LinearLayout(activity);
        headerActions.setGravity(Gravity.CENTER_VERTICAL);
        headerFocus = new ImageButton(activity);
        headerFocus.setScaleType(ImageView.ScaleType.FIT_CENTER);
        headerFocus.setPadding(dp(14), dp(14), dp(14), dp(14));
        headerFocus.setVisibility(View.GONE);
        headerFocus.setOnClickListener(v -> emit("focus", null));
        headerActions.addView(headerFocus, new LinearLayout.LayoutParams(dp(48), dp(48)));
        header.addView(headerActions, new LinearLayout.LayoutParams(-2, -2));
        page.addView(header);
        scroll = new ScrollView(activity) {
            @Override protected void onMeasure(int width, int height) {
                viewportHeight = MeasureSpec.getSize(height);
                super.onMeasure(width, height);
            }
        };
        scroll.setOnScrollChangeListener((view, x, y, oldX, oldY) -> placeMicrophoneCapsule());
        scroll.setFillViewport(true);
        content = column();
        content.setPadding(dp(16), dp(4), dp(16), dp(8));
        scroll.addView(content);
        page.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        feedback = label(14);
        feedback.setPadding(dp(16), dp(10), dp(16), dp(10));
        feedback.setMaxWidth(dp(480));
        feedback.setVisibility(View.GONE);
        feedback.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        FrameLayout.LayoutParams feedbackParams = new FrameLayout.LayoutParams(-2, -2, Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        feedbackParams.setMargins(dp(16), 0, dp(16), dp(12));
        pageFrame.addView(feedback, feedbackParams);
        feedback.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> placeFeedback());
        navigation = new LinearLayout(activity);
        navigationRail = new ScrollView(activity);
        navigationRail.setFillViewport(true);
        navigationRail.setVerticalScrollBarEnabled(false);
        positionNavigation();
        root.addView(screen, new FrameLayout.LayoutParams(-1, -1));
        screen.setVisibility(View.INVISIBLE);
        screen.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> {
            if (r - l != or - ol || b - t != ob - ot) {
                float density = activity.getResources().getDisplayMetrics().density;
                int windowWidth = r - l;
                int windowHeight = b - t;
                if (Build.VERSION.SDK_INT >= 30) {
                    WindowInsets insets = root.getRootWindowInsets();
                    if (insets != null) {
                        android.graphics.Insets bars = insets.getInsetsIgnoringVisibility(
                            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                        windowWidth = root.getWidth() - bars.left - bars.right;
                        windowHeight = root.getHeight() - bars.top - bars.bottom;
                    }
                }
                int widthDp = Math.round(windowWidth / density);
                int heightDp = Math.round(windowHeight / density);
                AdaptiveLayout.Workspace workspace = AdaptiveLayout.workspace(widthDp, heightDp);
                boolean landscape = workspace != AdaptiveLayout.Workspace.STACKED;
                boolean tablet = workspace == AdaptiveLayout.Workspace.TWO_PANE;
                boolean shortPortrait = !landscape && heightDp <= 620;
                int gutter = AdaptiveLayout.pageGutterDp(widthDp, workspace);
                boolean splitFiles = landscape || AdaptiveLayout.fittingColumns(widthDp - 2 * gutter, 20, 320, 2) == 2;
                if (wide != landscape || tabletWide != tablet || compact != shortPortrait || pageGutter != gutter || filesTwoPane != splitFiles) screen.post(() -> {
                    boolean structureChanged = wide != landscape || tabletWide != tablet || compact != shortPortrait;
                    if (!structureChanged && pageGutter == gutter && filesTwoPane == splitFiles) return;
                    filesTwoPane = splitFiles;
                    wide = landscape;
                    tabletWide = tablet;
                    compact = shortPortrait;
                    pageGutter = gutter;
                    if (structureChanged) positionNavigation();
                    if (latest != null) render(latest);
                });
                resizeDialog();
            }
        });
    }

    private void configureKeyboardViewport(boolean shortEntry) {
        shortKeyboard = shortEntry;
        page.setOrientation(shortEntry ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
        header.setLayoutParams(new LinearLayout.LayoutParams(shortEntry ? dp(184) : -1, shortEntry ? -1 : -2));
        headerActions.setVisibility(shortEntry ? View.GONE : View.VISIBLE);
        scroll.setLayoutParams(shortEntry ? new LinearLayout.LayoutParams(0, -1, 1)
            : new LinearLayout.LayoutParams(-1, 0, 1));
        header.setPadding(dp(pageGutter), dp(shortEntry ? 0 : 8), dp(shortEntry ? 0 : pageGutter), dp(shortEntry ? 0 : 8));
    }

    private final class WorkspaceHeader extends LinearLayout {
        private String connectionLabel = "";
        private CharSequence connectionWithLatency = "";
        private String chipStyle;
        private boolean chipTextDirty;

        WorkspaceHeader() { super(activity); }

        @Override protected void onMeasure(int widthMeasureSpec, int heightMeasureSpec) {
            if (shortKeyboard) {
                setOrientation(HORIZONTAL);
                LayoutParams identity = (LayoutParams) getChildAt(1).getLayoutParams();
                identity.width = 0;
                identity.weight = 1;
            } else if (headerActions.getChildCount() > 1) {
                TextView chip = (TextView) headerActions.getChildAt(0);
                int backWidth = headerBack.getVisibility() == View.VISIBLE ? dp(48) + dp(4) : 0;
                int available = Math.max(0, MeasureSpec.getSize(widthMeasureSpec) - getPaddingStart() - getPaddingEnd() - backWidth);
                int gap = dp(8);
                int identityWidth = (int) Math.ceil(Layout.getDesiredWidth(title.getText(), title.getPaint()));
                int subtitleWidth = (int) Math.ceil(Layout.getDesiredWidth(subtitle.getText(), subtitle.getPaint()));
                int chipPadding = chip.getCompoundPaddingStart() + chip.getCompoundPaddingEnd();
                int statusWidth = chipPadding + (int) Math.ceil(Layout.getDesiredWidth(connectionLabel, chip.getPaint()));
                int fullWidth = chipPadding + (int) Math.ceil(Layout.getDesiredWidth(connectionWithLatency, chip.getPaint()));
                int leadingWidth = Math.max(dp(128), Math.max(identityWidth, subtitleWidth));
                int actionWidth = headerFocus.getVisibility() == VISIBLE ? dp(48) : 0;
                boolean beside = AdaptiveLayout.fitsInline(available, leadingWidth, statusWidth + actionWidth, gap);
                int chipSpace = available - actionWidth - (beside ? leadingWidth + gap : 0);
                int maxWidth = Math.max(1, chipSpace);
                if (chip.getMaxWidth() != maxWidth) chip.setMaxWidth(maxWidth);
                CharSequence display = fullWidth <= chipSpace ? connectionWithLatency : connectionLabel;
                if (chipTextDirty || !TextUtils.equals(chip.getText(), display)) {
                    chip.setText(display);
                    chipTextDirty = false;
                }
                int maxLines = beside ? 1 : Integer.MAX_VALUE;
                if (chip.getMaxLines() != maxLines) chip.setMaxLines(maxLines);
                setOrientation(beside ? HORIZONTAL : VERTICAL);
                LayoutParams identity = (LayoutParams) getChildAt(1).getLayoutParams();
                identity.width = beside ? 0 : LayoutParams.MATCH_PARENT;
                identity.weight = beside ? 1 : 0;
                LayoutParams status = (LayoutParams) headerActions.getLayoutParams();
                status.gravity = beside ? Gravity.CENTER_VERTICAL : Gravity.END;
                status.setMarginStart(beside ? gap : 0);
                status.topMargin = beside ? 0 : gap;
            }
            super.onMeasure(widthMeasureSpec, heightMeasureSpec);
        }
    }

    private void sizeNavigationRail() {
        if (!wide) return;
        float widest = 0;
        for (int i = 0; i < navigation.getChildCount(); i++) {
            widest = Math.max(widest, desiredWidth((TextView) navigation.getChildAt(i)));
        }
        int width = Math.max(dp(AdaptiveLayout.NAV_RAIL_MIN_WIDTH_DP), Math.min(dp(AdaptiveLayout.NAV_RAIL_MAX_WIDTH_DP),
            (int) Math.ceil(widest) + navigation.getPaddingLeft() + navigation.getPaddingRight()));
        ViewGroup.LayoutParams layout = navigationRail.getLayoutParams();
        if (layout.width != width) {
            layout.width = width;
            navigationRail.setLayoutParams(layout);
        }
    }

    private void positionNavigation() {
        ViewGroup current = (ViewGroup) navigation.getParent();
        ViewGroup target = wide ? navigationRail : screen;
        if (current != target) {
            if (current != null) current.removeView(navigation);
            screen.removeView(navigationRail);
            navigation.removeAllViews();
            screen.setOrientation(wide ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
            LinearLayout.LayoutParams pageParams = wide ? new LinearLayout.LayoutParams(0, -1, 1)
                : new LinearLayout.LayoutParams(-1, 0, 1);
            if (pageFrame.getParent() == null) screen.addView(pageFrame, pageParams);
            else pageFrame.setLayoutParams(pageParams);
            if (wide) {
                navigationRail.addView(navigation, new ScrollView.LayoutParams(-1, -2));
                screen.addView(navigationRail, 0, new LinearLayout.LayoutParams(dp(AdaptiveLayout.NAV_RAIL_MIN_WIDTH_DP), -1));
            } else screen.addView(navigation, new LinearLayout.LayoutParams(-1, -2));
        }
        header.setMinimumHeight(dp(tabletWide ? 72 : wide ? 64 : 70));
        title.setTextSize(wide ? 22 : compact ? 23 : 26);
        navigation.setOrientation(wide ? LinearLayout.VERTICAL : LinearLayout.HORIZONTAL);
        navigation.setGravity(tabletWide ? Gravity.TOP : wide ? Gravity.CENTER_VERTICAL : Gravity.CENTER);
        navigation.setPadding(dp(wide ? 5 : 16), dp(tabletWide ? 72 : 6), dp(wide ? 5 : 16), dp(8));
        for (int i = 0; wide && i < navigation.getChildCount(); i++) {
            View child = navigation.getChildAt(i);
            LinearLayout.LayoutParams params = (LinearLayout.LayoutParams) child.getLayoutParams();
            int margin = i == navigation.getChildCount() - 1 ? 0 : dp(tabletWide ? 12 : 18);
            if (params.bottomMargin != margin) {
                params.bottomMargin = margin;
                child.setLayoutParams(params);
            }
        }
    }

    private int dp(float value) { return Math.round(value * activity.getResources().getDisplayMetrics().density); }
    private static boolean hasComposingText(Editable text) {
        if (text == null) return false;
        int start = BaseInputConnection.getComposingSpanStart(text);
        int end = BaseInputConnection.getComposingSpanEnd(text);
        return start >= 0 && end > start;
    }
    private int ink() { return dark ? 0xffedf0f7 : 0xff1a202c; }
    private int muted() { return dark ? 0xffa3adbf : 0xff5f6b7e; }
    private int surface() { return dark ? 0xff1b1e26 : Color.WHITE; }
    private int tonal() { return dark ? 0xff252a35 : 0xffeceff5; }
    private int edge() { return dark ? 0xff303642 : 0xffe6e9f0; }
    private int background() { return dark ? 0xff111318 : 0xfff5f6fa; }
    private int accent() { return dark ? 0xffa5bdff : 0xff2868e8; }
    private int accentSoft() { return dark ? 0xff283b62 : 0xffe6edff; }
    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(activity);
        layout.setOrientation(LinearLayout.VERTICAL);
        return layout;
    }
    private TextView label(int size) {
        TextView view = new TextView(activity);
        view.setTextSize(size);
        return view;
    }
    private GradientDrawable shape(int color, int radius) {
        GradientDrawable result = new GradientDrawable();
        result.setColor(color);
        result.setCornerRadius(dp(radius));
        return result;
    }
    private RippleDrawable ripple(int color) {
        return ripple(color, 16);
    }
    private RippleDrawable ripple(int color, int radius) {
        int pressed = dark ? 0x30ffffff : 0x240051dc;
        int focused = dark ? 0x28a5bdff : 0x202868e8;
        int hover = dark ? 0x1affffff : 0x142868e8;
        ColorStateList states = new ColorStateList(new int[][]{
            {android.R.attr.state_pressed}, {android.R.attr.state_focused}, {android.R.attr.state_hovered}, {}
        }, new int[]{pressed, focused, hover, pressed});
        return new RippleDrawable(states, shape(color, radius), shape(Color.WHITE, radius));
    }
    private void setText(TextView view, String value) {
        if (!value.contentEquals(view.getText())) view.setText(value);
    }
    private void setSupportingText(TextView view, String title, String supporting) {
        if (supporting.isEmpty()) { setText(view, title); return; }
        String combined = title + "\n" + supporting;
        SpannableString styled = new SpannableString(combined);
        styled.setSpan(new ForegroundColorSpan(muted()), title.length() + 1, combined.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        styled.setSpan(new RelativeSizeSpan(.86f), title.length() + 1, combined.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        view.setText(styled);
    }
    private void markHeading(View view) {
        if (Build.VERSION.SDK_INT >= 28) view.setAccessibilityHeading(true);
    }
    private void emit(String id, Object value) {
        if ("send".equals(id) || "pair".equals(id)) {
            String field = "send".equals(id) ? "edit:draft" : "edit:code";
            View host = sheetContent != null ? sheetContent : content;
            SessionEdit edit = host.findViewWithTag(field);
            if (edit != null && hasComposingText(edit.getText())) {
                Toast.makeText(activity, AppLanguage.text(activity, R.string.ui_confirm_input_candidate_first), Toast.LENGTH_SHORT).show();
                return;
            }
        }
        host.action(id, value);
    }

    void render(JSONObject state) {
        if (activity.isFinishing() || activity.isDestroyed()) return;
        boolean nextDark = state.optBoolean("dark");
        boolean themeChanged = latest == null || dark != nextDark;
        latest = state;
        dark = nextDark;
        if (themeChanged) {
            screen.setBackgroundColor(background());
            title.setTextColor(ink());
            subtitle.setTextColor(muted());
            navigation.setBackgroundColor(dark ? 0xff181c24 : 0xfff0f2f8);
            feedback.setTextColor(surface());
            feedback.setBackground(shape(ink(), 12));
        }
        JSONObject currentMicrophone = state.optJSONObject("microphone");
        microphoneState = currentMicrophone == null ? new JSONObject() : currentMicrophone;
        boolean locked = microphoneState.optBoolean("locked");
        subtitle.setVisibility(locked ? View.INVISIBLE : View.VISIBLE);
        microphoneStatus.setVisibility(locked ? View.VISIBLE : View.GONE);
        microphoneStatus.stopButton().setVisibility(locked ? View.VISIBLE : View.GONE);
        if (locked) microphoneStatus.update(microphoneState, dark);
        screen.setVisibility(View.VISIBLE);
        setText(title, state.optString("title"));
        setText(subtitle, state.optString("subtitle", ""));
        JSONObject connection = state.optJSONObject("connection");
        if (connection != null) {
            if (headerActions.getChildCount() == 1) headerActions.addView(create(connection, headerActions), 0, new LinearLayout.LayoutParams(-2, -2));
            update(headerActions.getChildAt(0), connection);
        }
        String dest = state.optString("destination");
        boolean headerBackVisible = "keyboard".equals(dest) || "windows".equals(dest);
        if (headerBack.getVisibility() != (headerBackVisible ? View.VISIBLE : View.GONE))
            headerBack.setVisibility(headerBackVisible ? View.VISIBLE : View.GONE);
        if (themeChanged || headerBackIcon.getDrawable() == null)
            headerBackIcon.setImageDrawable(NativeIcon.of("back", ink(), dp(22)));
        headerBack.setContentDescription(AppLanguage.text(activity, R.string.ui_back_to_remote));
        int gutter = pageGutter;
        if (!dest.equals(content.getTag())) {
            content.setTag(dest);
            scroll.scrollTo(0, 0);
        }
        JSONArray controls = state.optJSONArray("controls");
        if (!"remote".equals(dest)) headerFocus.setVisibility(View.GONE);
        if ("remote".equals(dest) && controls != null) controls = remoteLayout(controls, state.optBoolean("side"));
        else if ("settings".equals(dest) && controls != null) controls = settingsLayout(controls);
        else if ("files".equals(dest) && controls != null) controls = filesLayout(controls);
        else if ("keyboard".equals(dest) && controls != null) controls = new JSONArray().put(group("column", "keyboard-column", controls));
        JSONObject first = controls == null ? null : controls.optJSONObject(0);
        boolean splitPad = wide && "remote".equals(dest) && first != null && "landscape".equals(first.optString("id"));
        if (!splitPad && header.getParent() != page) {
            if (header.getParent() instanceof ViewGroup previous) previous.removeView(header);
            page.addView(header, 0, new LinearLayout.LayoutParams(-1, -2));
        }
        int headerInset = splitPad ? 0 : gutter;
        int headerVertical = splitPad || shortKeyboard ? 0 : dp(8);
        header.setPadding(dp(headerInset), headerVertical, dp(shortKeyboard ? 0 : headerInset), headerVertical);
        content.setPadding(dp(gutter), dp(splitPad ? 8 : "remote".equals(dest) ? 0 : 4), dp(gutter),
            dp(tabletWide ? gutter : "remote".equals(dest) ? 8 : 24));
        boolean hadTransfer = content.findViewWithTag("column:transfer-card") != null;
        reconcile(content, controls, false);
        if ("files".equals(dest) && !hadTransfer && content.findViewWithTag("column:transfer-card") != null)
            scroll.post(() -> scroll.scrollTo(0, 0));
        reconcile(navigation, state.optJSONArray("nav"), !wide);
        sizeNavigationRail();
        navigation.setVisibility(navigation.getChildCount() == 0 ? View.GONE : state.optBoolean("keepDialPosition") ? View.INVISIBLE : View.VISIBLE);
        navigationRail.setVisibility(navigation.getVisibility());
        setText(feedback, state.optString("feedback"));
        feedback.setVisibility(feedback.length() > 0 && state.isNull("sheetKey") ? View.VISIBLE : View.GONE);
        if (feedback.getVisibility() == View.VISIBLE) placeFeedback();
        renderSheet(state);
        updateMicrophoneCapsule("remote".equals(dest) && currentMicrophone != null && microphoneState.optBoolean("expanded"));
    }

    private void updateMicrophoneCapsule(boolean open) {
        if (!open) {
            if (microphoneCapsule != null) microphoneCapsule.setVisibility(View.GONE);
            return;
        }
        if (microphoneCapsule == null) {
            microphoneCapsule = microphoneUi.new Capsule();
            microphoneCapsule.setVisibility(View.INVISIBLE);
            root.addView(microphoneCapsule, new FrameLayout.LayoutParams(dp(330), -2));
            microphoneCapsule.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> placeMicrophoneCapsule());
        }
        boolean appearing = microphoneCapsule.getVisibility() != View.VISIBLE;
        microphoneCapsule.setVisibility(View.VISIBLE);
        microphoneCapsule.update(microphoneState, dark);
        placeMicrophoneCapsule();
        if (appearing) {
            microphoneCapsule.setAlpha(0f);
            microphoneCapsule.animate().alpha(1f).setDuration(180).start();
        }
    }

    private void placeFeedback() {
        View key = content.findViewWithTag("mic:mic");
        if (key == null || feedback.getHeight() == 0 || !key.isShown()) {
            feedback.setTranslationY(0);
            return;
        }
        android.graphics.Rect area = new android.graphics.Rect();
        key.getDrawingRect(area);
        pageFrame.offsetDescendantRectToMyCoords(key, area);
        int top = Math.max(0, area.top - feedback.getHeight() - dp(8));
        feedback.setTranslationY(Math.min(0, top - feedback.getTop()));
    }

    boolean showMicrophonePermissionHint(boolean granted, boolean retry, Runnable action) {
        View key = content.findViewWithTag("mic:mic");
        if (key == null || !key.isShown() || !(key.getParent() instanceof View tools)) return false;
        if (permissionHint != null) permissionHint.dismiss();
        LinearLayout card = new LinearLayout(activity);
        card.setGravity(Gravity.CENTER_VERTICAL);
        card.setPadding(dp(12), dp(4), dp(4), dp(4));
        TextView message = label(14);
        message.setTextColor(ink());
        message.setText(AppLanguage.text(activity, granted ? R.string.ui_microphone_permission_granted_press_and_hold_again : R.string.ui_mic_access_off));
        card.addView(message, new LinearLayout.LayoutParams(0, -2, 1));
        int width = Math.min(dp(360), tools.getWidth());
        PopupWindow hint = new PopupWindow(card, width, -2, true);
        permissionHint = hint;
        if (!granted) {
            Button button = new Button(activity);
            button.setAllCaps(false);
            button.setTextColor(accent());
            button.setText(AppLanguage.text(activity, retry ? R.string.ui_permission_retry : R.string.ui_permission_settings));
            button.setBackground(ripple(Color.TRANSPARENT));
            button.setMinimumHeight(dp(48));
            button.setOnClickListener(v -> { hint.dismiss(); action.run(); });
            card.addView(button);
        } else card.setMinimumHeight(dp(48));
        hint.setBackgroundDrawable(shape(surface(), 16));
        hint.setOutsideTouchable(true);
        hint.setElevation(dp(6));
        hint.setOnDismissListener(() -> { if (permissionHint == hint) permissionHint = null; });
        card.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        hint.showAsDropDown(tools, 0, -tools.getHeight() - card.getMeasuredHeight() - dp(6), Gravity.CENTER_HORIZONTAL);
        message.announceForAccessibility(message.getText());
        if (granted) card.postDelayed(hint::dismiss, 2600);
        return true;
    }

    private void placeMicrophoneCapsule() {
        if (microphoneCapsule == null || microphoneCapsule.getVisibility() != View.VISIBLE) return;
        View key = content.findViewWithTag("mic:mic");
        if (key == null || !(key.getParent() instanceof View)) return;
        View tools = (View) key.getParent();
        int[] rootAt = new int[2], toolsAt = new int[2];
        root.getLocationOnScreen(rootAt);
        tools.getLocationOnScreen(toolsAt);
        int width = wide ? Math.min(dp(330), Math.max(tools.getWidth(), dp(235))) : tools.getWidth();
        FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) microphoneCapsule.getLayoutParams();
        if (width > 0 && lp.width != width) { lp.width = width; microphoneCapsule.setLayoutParams(lp); }
        int x = toolsAt[0] - rootAt[0];
        if (wide && latest != null && !latest.optBoolean("side")) x += tools.getWidth() - width;
        microphoneCapsule.setTranslationX(Math.max(dp(8), x) - microphoneCapsule.getLeft());
        int gap = tabletWide ? 8 : wide ? 20 : Math.max(8, 86 - Math.round(tools.getHeight() / activity.getResources().getDisplayMetrics().density));
        int top = Math.max(dp(58), toolsAt[1] - rootAt[1] - microphoneCapsule.getHeight() - dp(gap));
        microphoneCapsule.setTranslationY(top - microphoneCapsule.getTop());
    }

    private JSONArray remoteLayout(JSONArray items, boolean right) {
        JSONArray top = new JSONArray(), pad = new JSONArray(), tools = new JSONArray();
        JSONArray media = new JSONArray();
        boolean directional = false;
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            String id = item.optString("id");
            if ("dpad".equals(id)) directional = true;
            if ("pad".equals(id) || "clicks".equals(id) || id.startsWith("dpad")) pad.put(item);
            else if ("volume".equals(id) || "playback".equals(id)) media.put(item);
            else if ("tools".equals(id) || "handset-tools".equals(id)) tools.put(item);
            else if ("session".equals(id)) {
                JSONArray children = item.optJSONArray("children");
                JSONArray actions = new JSONArray();
                if (wide && children.optJSONObject(1) != null) actions.put(children.optJSONObject(1));
                JSONObject primary = children.optJSONObject(0);
                boolean inline = !wide && primary != null && "focus".equals(primary.optString("id"));
                headerFocus.setVisibility(inline ? View.VISIBLE : View.GONE);
                if (inline) {
                    String icon = primary.optString("icon");
                    String style = icon + dark;
                    if (!style.equals(headerFocus.getTag())) {
                        headerFocus.setTag(style);
                        headerFocus.setImageDrawable(NativeIcon.of(icon, muted(), dp(20)));
                        headerFocus.setBackground(ripple(Color.TRANSPARENT, 24));
                    }
                    String label = primary.optString("label");
                    headerFocus.setContentDescription(label);
                    headerFocus.setTooltipText(label);
                } else if (primary != null) actions.put(primary);
                if (actions.length() > 0) top.put(group("row", "session", actions));
            }
            else top.put(item);
        }
        JSONObject padColumn = group("column", directional ? "dial-column" : "pad-column", pad);
        JSONObject toolsRow = tools.optJSONObject(0);
        JSONArray side = new JSONArray();
        if (wide) {
            side.put(group("header", "remote-header", new JSONArray()));
            for (int i = 0; i < top.length(); i++) side.put(top.optJSONObject(i));
        }
        if (directional) {
            if (!wide) side.put(pad.optJSONObject(0));
            side.put(toolsRow);
            for (int i = 0; i < media.length(); i++) {
                JSONObject row = media.optJSONObject(i);
                side.put(group("row", "rocker-" + row.optString("id"), row.optJSONArray("children")));
            }
            if (!wide) return top.put(group("column", "handset-portrait", side));
        } else {
            if (!wide) return top.put(padColumn).put(toolsRow).put(group("row", "media", media));
            if (tabletWide) {
                JSONArray buttons = toolsRow.optJSONArray("children");
                JSONArray upper = new JSONArray(), lower = new JSONArray();
                for (int i = 0; i < buttons.length(); i++) {
                    (i < 2 ? upper : lower).put(buttons.optJSONObject(i));
                }
                side.put(group("row", "tools-upper", upper));
                side.put(group("row", "tools-lower", lower));
            } else {
                side.put(toolsRow);
            }
            for (int i = 0; i < media.length(); i++) side.put(media.optJSONObject(i));
        }
        JSONObject toolColumn = group("column", "tool-column", side);
        return new JSONArray().put(group("row", "landscape", new JSONArray().put(right ? toolColumn : padColumn).put(right ? padColumn : toolColumn)));
    }

    private JSONArray settingsLayout(JSONArray items) {
        JSONArray result = new JSONArray();
        JSONArray section = null, sectionContent = null;
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            String id = item.optString("id");
            if ("heading".equals(item.optString("kind"))) {
                section = new JSONArray().put(item);
                result.put(group("column", "section-" + id, section));
                sectionContent = new JSONArray();
                section.put(group("column", "settings-" + id, sectionContent));
            } else if ("shortcut-tip".equals(id)) section.put(item);
            else if ("settings-footnote".equals(id)) result.put(item);
            else if (sectionContent != null) sectionContent.put(settingItem(item));
        }
        if (tabletWide && result.length() >= 4) {
            return new JSONArray().put(group("row", "grid-settings-top",
                new JSONArray().put(result.optJSONObject(0)).put(result.optJSONObject(1))))
                .put(group("row", "grid-settings-bottom", new JSONArray().put(result.optJSONObject(2)).put(result.optJSONObject(3))))
                .put(result.optJSONObject(4));
        }
        return result;
    }

    private JSONObject settingItem(JSONObject item) {
        if ("button".equals(item.optString("kind"))) {
            try { return new JSONObject(item.toString()).put("kind", "setting-button"); }
            catch (JSONException ignored) {}
        }
        return item;
    }

    private JSONArray filesLayout(JSONArray items) {
        JSONArray actions = new JSONArray(), library = new JSONArray(), history = new JSONArray();
        JSONObject upload = null, receive = null, header = null, progress = null, result = null;
        boolean inLibrary = false;
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            String id = item.optString("id");
            if ("upload".equals(id)) upload = item;
            else if ("receive".equals(id) && i + 2 < items.length()) {
                JSONObject path = items.optJSONObject(++i);
                JSONObject picker = items.optJSONObject(++i);
                JSONObject caption = group("column", "folder-copy", new JSONArray().put(item).put(path));
                receive = group("row", "folder-card", new JSONArray().put(caption).put(picker));
            } else if ("shared".equals(id) && i + 1 < items.length()) {
                inLibrary = true;
                header = group("row", "files-header", new JSONArray().put(item).put(items.optJSONObject(++i)));
            } else if (inLibrary || "files-status".equals(id)) library.put(item);
            else if (id.startsWith("history-") || "clear-history".equals(id)) history.put(item);
            else if ("transfer-card".equals(id)) progress = item;
            else if ("transfer".equals(id)) result = item;
            else actions.put(item);
        }
        if (progress != null) actions.put(progress);
        if (upload != null) actions.put(upload);
        if (receive != null) actions.put(receive);
        if (result != null) actions.put(result);
        JSONArray librarySections = new JSONArray();
        if (header != null) librarySections.put(header);
        if (library.length() > 0) librarySections.put(group("column", "file-library-card", library));
        if (history.length() > 0) (filesTwoPane ? actions : librarySections).put(group("column", "file-history-card", history));
        JSONObject actionColumn = group("column", "file-actions-column", actions);
        JSONObject libraryColumn = group("column", "file-library-column", librarySections);
        JSONArray sections = new JSONArray().put(actionColumn).put(libraryColumn);
        return filesTwoPane ? new JSONArray().put(group("row", "files-columns", sections)) : sections;
    }

    private JSONObject group(String kind, String id, JSONArray children) {
        try { return new JSONObject().put("kind", kind).put("id", id).put("children", children); }
        catch (JSONException impossible) { throw new IllegalStateException(impossible); }
    }

    private void reconcile(LinearLayout parent, JSONArray items, boolean horizontal) {
        int count = items == null ? 0 : items.length();
        boolean fileColumn = "column:file-actions-column".equals(parent.getTag())
            || "column:file-library-column".equals(parent.getTag());
        java.util.HashSet<String> tags = new java.util.HashSet<>();
        for (int index = 0; index < count; index++) {
            JSONObject item = items.optJSONObject(index);
            if (item != null) tags.add(item.optString("kind") + ":" + item.optString("id"));
        }
        for (int index = parent.getChildCount() - 1; index >= 0; index--) {
            if (!tags.contains(parent.getChildAt(index).getTag())) parent.removeViewAt(index);
        }
        for (int index = 0; index < count; index++) {
            JSONObject item = items.optJSONObject(index);
            if (item == null) continue;
            String tag = item.optString("kind") + ":" + item.optString("id");
            View view = parent.getChildAt(index);
            if (view == null || !tag.equals(view.getTag())) {
                View existing = null;
                for (int later = index + 1; later < parent.getChildCount(); later++) {
                    if (tag.equals(parent.getChildAt(later).getTag())) {
                        existing = parent.getChildAt(later);
                        parent.removeViewAt(later);
                        break;
                    }
                }
                view = existing == null ? create(item, parent) : existing;
                LinearLayout.LayoutParams layout = params(item, horizontal);
                applySpacing(layout, parent, item, index, count);
                parent.addView(view, index, layout);
            }
            if (fileColumn) {
                LinearLayout.LayoutParams lp = (LinearLayout.LayoutParams) view.getLayoutParams();
                int gap = index == 0 ? 0 : dp("file-library-card".equals(item.optString("id")) ? 4 : 12);
                if (lp.topMargin != gap || lp.bottomMargin != 0) {
                    lp.topMargin = gap;
                    lp.bottomMargin = 0;
                    view.setLayoutParams(lp);
                }
            }
            update(view, item);
        }
        while (parent.getChildCount() > count) parent.removeViewAt(parent.getChildCount() - 1);
    }

    private void applySpacing(LinearLayout.LayoutParams layout, LinearLayout parent, JSONObject item, int index, int count) {
        String group = String.valueOf(parent.getTag());
        String id = item.optString("id");
        if ("strip-items".equals(group)) {
            layout.width = -2;
            layout.weight = 0;
            layout.setMargins(index == 0 ? 0 : dp(6), 0, 0, 0);
        } else if ("row:draft-meta".equals(group)) {
            if ("send".equals(id)) { layout.width = dp(52); layout.weight = 0; }
            layout.setMargins(index == 0 ? 0 : dp(8), 0, 0, 0);
        } else if ("column:keyboard-compose".equals(group) || "column:keyboard-shortcuts".equals(group)) {
            layout.setMargins(0, index == 0 ? 0 : dp(6), 0, 0);
        } else if ("row:window-actions".equals(group)) {
            if ("window-actions-spacer".equals(id)) { layout.width = 0; layout.weight = 1; }
            else { layout.width = dp(52); layout.weight = 0; }
            layout.setMargins(index == 0 ? 0 : dp(4), 0, 0, 0);
        } else if ("column:handset-portrait".equals(group) || ("column:tool-column".equals(group)
            && ("handset-tools".equals(id) || id.startsWith("rocker-")))) {
            boolean dial = "dpad".equals(id);
            boolean shortcuts = "handset-tools".equals(id);
            layout.height = dial ? -2 : dp(shortcuts ? (wide ? 56 : 68) : (wide ? 52 : 62));
            layout.weight = 0;
            layout.setMargins(0, index == 0 ? 0 : dp(shortcuts ? 16 : 10), 0, 0);
        } else if ("row:handset-tools".equals(group) || group.startsWith("row:rocker-")) {
            layout.height = ViewGroup.LayoutParams.MATCH_PARENT;
            layout.setMargins(index > 0 && "row:handset-tools".equals(group) ? dp(8) : 0, 0, 0, 0);
        } else if (group.startsWith("row:more-") || group.startsWith("row:window-") || "row:media".equals(group)
            || group.startsWith("row:tools")) {
            int gap = group.startsWith("row:more-") ? MORE_GAP_DP : 8;
            layout.setMargins(index == 0 ? 0 : dp(gap), 0, 0, 0);
            if (group.startsWith("row:more-") || group.startsWith("row:tools")) layout.height = ViewGroup.LayoutParams.MATCH_PARENT;
        } else if ("row:session".equals(group)) {
            int end = "pad-side".equals(id) ? dp(6) : 0;
            layout.setMargins(0, 0, end, 0);
        } else if ("row:files-columns".equals(group)) {
            layout.setMargins(index == 0 ? 0 : dp(20), 0, 0, 0);
        } else if ("row:landscape".equals(group)) {
            layout.setMargins(0, 0, 0, 0);
        } else if (group.startsWith("row:grid-settings-")) {
            layout.setMargins(index == 0 ? 0 : dp(16), 0, 0, dp(12));
        } else if ("column:tool-column".equals(group) || "column:transfer-card".equals(group)) {
            layout.setMargins(0, index == 0 ? 0 : dp(8), 0, 0);
        } else if ("column:folder-copy".equals(group)) {
            layout.setMargins(0, index == 0 ? 0 : dp(4), 0, 0);
        } else if (group.startsWith("row:file-")) {
            layout.setMargins(index == 0 ? 0 : dp(8), 0, 0, 0);
        } else if ("column:pad-column".equals(group)) {
            layout.topMargin = 0;
            layout.bottomMargin = "dpad".equals(id) ? dp(6) : 0;
        } else if (group.startsWith("column:settings-") || "column:connection-hero".equals(group)
            || "column:connection-metrics".equals(group) || "column:file-library-card".equals(group)
            || "column:file-history-card".equals(group)) {
            layout.topMargin = 0;
            layout.bottomMargin = 0;
        }
        if (parent == sheetContent) {
            int gap = "more-slots".equals(id) ? 18 : "connection".equals(sheetKey) ? 12 : 8;
            if (compactMoreSheet) gap = "more-slots".equals(id) ? 8 : 4;
            layout.topMargin = index == 0 || item.has("selected") ? 0 : dp(gap);
            layout.bottomMargin = 0;
        }
        if (parent == content && "file-library-column".equals(id)) layout.topMargin = dp(16);
        if (parent == content && "remote".equals(parent.getTag())) {
            layout.setMargins(0, index == 0 ? 0 : dp(compact ? 4 : 8), 0, 0);
        }
        if ("row:folder-card".equals(group) || "row:files-header".equals(group)) {
            layout.leftMargin = 0;
            layout.rightMargin = 0;
        }
        if (parent == navigation && wide && index == count - 1) layout.bottomMargin = 0;
    }

    private LinearLayout.LayoutParams params(JSONObject item, boolean horizontal) {
        String id = item.optString("id");
        boolean grows = "pad".equals(id) || "dpad".equals(id) || "landscape".equals(id) || "pad-column".equals(id);
        float columnWeight = "pad-column".equals(id) ? (tabletWide ? 1.65f : 1.1f) : "file-library-column".equals(id) ? 1.16f : 1f;
        LinearLayout.LayoutParams lp = horizontal
            ? new LinearLayout.LayoutParams(0, id.endsWith("-column") ? -1 : -2, columnWeight)
            : new LinearLayout.LayoutParams(-1, grows ? 0 : -2, grows ? 1 : 0);
        if ("dpad".equals(id) || "dial-column".equals(id) || "handset-portrait".equals(id)) {
            if (!horizontal) { lp.height = -2; lp.weight = 1; }
            lp.gravity = Gravity.CENTER;
        }
        if ("handset-tools".equals(id) || "media".equals(id)) lp.gravity = Gravity.CENTER_HORIZONTAL;
        if ("focus".equals(id) || "pad-side".equals(id) || "offline-change".equals(id)) {
            lp.width = -2;
            lp.weight = 0;
        }
        if ("folder-copy".equals(id)) lp.height = -2;
        if ("keyboard-column".equals(id)) lp.gravity = Gravity.CENTER_HORIZONTAL;
        if ("pick-folder".equals(id) || "reload".equals(id) || id.startsWith("save-") || "mic-stop".equals(id)
            || "character-count".equals(id) || "connection-latency".equals(id)) {
            lp.width = -2;
            lp.weight = 0;
        }
        lp.setMargins(dp(horizontal ? 3 : 0), dp(3), dp(horizontal ? 3 : 0), dp(3));
        if (id.startsWith("section-")) {
            int outer = tabletWide ? pageGutter / 2 : 0;
            lp.setMargins(dp(outer), 0, dp(outer), dp(12));
        }
        if (id.startsWith("nav-") && wide) lp.setMargins(0, 0, 0, dp(tabletWide ? 12 : 18));
        return lp;
    }

    private View create(JSONObject item, LinearLayout parent) {
        String kind = item.optString("kind"), id = item.optString("id");
        View view;
        switch (kind) {
            case "header":
                if (header.getParent() instanceof ViewGroup previous) previous.removeView(header);
                view = header;
                break;
            case "row":
            case "column":
                LinearLayout group = new DividedLayout(id);
                group.setOrientation(LinearLayout.VERTICAL);
                if ("row".equals(kind)) {
                    group.setOrientation(LinearLayout.HORIZONTAL);
                    group.setBaselineAligned(false);
                    if ("window-actions".equals(id)) group.setGravity(Gravity.END | Gravity.CENTER_VERTICAL);
                    if ("draft-meta".equals(id)) group.setGravity(Gravity.CENTER_VERTICAL);
                }
                view = group;
                break;
            case "strip":
                if ("column:keyboard-shortcuts".equals(parent.getTag())) {
                    view = new KeyboardGrid();
                    break;
                }
                HorizontalScrollView strip = new KeyStrip();
                strip.setHorizontalScrollBarEnabled(false);
                strip.setHorizontalFadingEdgeEnabled(true);
                strip.setFadingEdgeLength(dp(12));
                LinearLayout items = new LinearLayout(activity);
                items.setTag("strip-items");
                strip.addView(items, new HorizontalScrollView.LayoutParams(-2, -2));
                view = strip;
                break;
            case "switch":
                CompoundButton toggle = new PreferenceToggle();
                toggle.setOnClickListener(v -> emit(id, toggle.isChecked()));
                view = toggle;
                break;
            case "slider":
                LinearLayout slider = column();
                slider.setPadding(dp(16), dp(12), dp(16), dp(12));
                LinearLayout sliderTitle = new LinearLayout(activity);
                sliderTitle.setGravity(Gravity.CENTER_VERTICAL);
                sliderTitle.addView(label(14), new LinearLayout.LayoutParams(0, -2, 1));
                TextView amount = label(13);
                amount.setPadding(dp(9), dp(3), dp(9), dp(3));
                sliderTitle.addView(amount);
                slider.addView(sliderTitle);
                SeekBar seek = new SeekBar(activity);
                seek.setPaddingRelative(dp(8), 0, dp(8), 0);
                seek.setMax(25);
                seek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                    @Override public void onProgressChanged(SeekBar bar, int value, boolean user) {
                        if (user) emit(id, (5 + value) / 10.0);
                    }
                    @Override public void onStartTrackingTouch(SeekBar bar) {}
                    @Override public void onStopTrackingTouch(SeekBar bar) {}
                });
                slider.addView(seek, new LinearLayout.LayoutParams(-1, dp(AdaptiveLayout.TOUCH_TARGET_DP)));
                LinearLayout scale = new LinearLayout(activity);
                scale.addView(label(12), new LinearLayout.LayoutParams(0, -2, 1));
                scale.addView(label(12), new LinearLayout.LayoutParams(-2, -2));
                slider.addView(scale, new LinearLayout.LayoutParams(-1, -2));
                view = slider;
                break;
            case "edit":
                SessionEdit edit = new SessionEdit();
                boolean numeric = "code".equals(id);
                edit.setTextSize(17);
                edit.setSingleLine(numeric);
                edit.setInputType(numeric ? InputType.TYPE_CLASS_NUMBER : InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES);
                if (!numeric) edit.setGravity(Gravity.TOP | Gravity.START);
                edit.setMinLines(numeric ? 1 : 2);
                edit.setMaxLines(numeric ? 1 : "column:keyboard-entry".equals(parent.getTag()) ? Integer.MAX_VALUE : 4);
                edit.setFilters(new InputFilter[]{new InputFilter.LengthFilter(numeric ? 6 : 1000)});
                edit.setImeOptions(numeric ? EditorInfo.IME_ACTION_DONE : EditorInfo.IME_ACTION_SEND | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
                edit.setOnEditorActionListener((v, action, event) -> {
                    if (action != (numeric ? EditorInfo.IME_ACTION_DONE : EditorInfo.IME_ACTION_SEND)) return false;
                    // Enter only confirms an IME candidate while composing; never send it to the PC.
                    if (!numeric && hasComposingText(edit.getText())) return true;
                    emit(numeric ? "pair" : "send", null);
                    return true;
                });
                edit.addTextChangedListener(new TextWatcher() {
                    @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
                    @Override public void onTextChanged(CharSequence s, int start, int before, int count) {
                        if (!editing) {
                            try { emit(id, new JSONObject().put("text", s.toString()).put("revision", ++edit.revision)); }
                            catch (JSONException ignored) {}
                        }
                    }
                    @Override public void afterTextChanged(Editable s) {}
                });
                view = edit;
                break;
            case "progress":
                ProgressBar progress = new ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal);
                progress.setMax(100);
                view = progress;
                break;
            case "window":
                LinearLayout windowRow = new LinearLayout(activity);
                windowRow.setOrientation(LinearLayout.HORIZONTAL);
                windowRow.setGravity(Gravity.CENTER_VERTICAL);
                windowRow.setMinimumHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
                windowRow.setPadding(dp(12), dp(8), dp(12), dp(8));
                windowRow.setFocusable(true);
                ImageView windowIcon = new ImageView(activity);
                windowIcon.setScaleType(ImageView.ScaleType.FIT_CENTER);
                windowRow.addView(windowIcon, new LinearLayout.LayoutParams(dp(30), dp(30)));
                LinearLayout windowCopy = column();
                windowCopy.addView(label(14));
                windowCopy.addView(label(12));
                LinearLayout.LayoutParams copyParams = new LinearLayout.LayoutParams(0, -2, 1);
                copyParams.setMarginStart(dp(12));
                windowRow.addView(windowCopy, copyParams);
                TextView windowState = label(12);
                windowState.setGravity(Gravity.END);
                windowRow.addView(windowState, new LinearLayout.LayoutParams(-2, -2));
                windowRow.setOnClickListener(v -> emit(id, null));
                view = windowRow;
                break;
            case "dpad":
                view = new DPadView();
                break;
            case "pad":
            case "mic":
                TextView target = "pad".equals(kind) ? new TouchPadView() : microphoneUi.new Key();
                target.setGravity(Gravity.CENTER);
                target.setMinHeight(dp("pad".equals(kind) ? 148 : 48));
                if ("pad".equals(id) && Build.VERSION.SDK_INT >= 29) {
                    target.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> {
                        int edge = Math.min(dp(22), (r - l) / 6);
                        int band = Math.min(dp(190), b - t);
                        int top = (b - t - band) / 2;
                        v.setSystemGestureExclusionRects(java.util.Arrays.asList(
                            new Rect(0, top, edge, top + band), new Rect(r - l - edge, top, r - l, top + band)));
                    });
                }
                target.setOnClickListener(v -> emit(id, null));
                if ("mic".equals(id)) target.setOnKeyListener((v, key, event) -> {
                    if (key != KeyEvent.KEYCODE_ENTER && key != KeyEvent.KEYCODE_DPAD_CENTER && key != KeyEvent.KEYCODE_SPACE) return false;
                    if (event.getRepeatCount() == 0) emit(id, event.getAction() == KeyEvent.ACTION_DOWN);
                    return true;
                });
                target.setOnTouchListener((v, event) -> {
                    if (!v.isEnabled()) return false;
                    v.getParent().requestDisallowInterceptTouchEvent(true);
                    forwardPointers(v, event, id);
                    return true;
                });
                view = target;
                break;
            case "button":
                if (String.valueOf(parent.getTag()).startsWith("row:more-")) {
                    view = new MoreTile(id);
                    break;
                }
                Button button = new ControlButton();
                button.setAllCaps(false);
                button.setStateListAnimator(null);
                button.setElevation(0);
                button.setTextSize(14);
                button.setMinHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
                button.setMinimumHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
                button.setMinWidth(0);
                button.setMinimumWidth(0);
                button.setOnClickListener(v -> emit(id, null));
                view = button;
                break;
            case "setting-button":
                LinearLayout setting = new SettingRow();
                setting.setGravity(Gravity.CENTER_VERTICAL);
                setting.setMinimumHeight(dp(52));
                setting.setPadding(dp(16), dp(8), dp(16), dp(8));
                setting.setOnClickListener(v -> emit(id, null));
                setting.setFocusable(true);
                TextView settingLabel = label(14), settingValue = label(13);
                settingValue.setGravity(Gravity.END);
                settingLabel.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
                settingValue.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
                setting.addView(settingLabel, new LinearLayout.LayoutParams(0, -2, 1));
                LinearLayout.LayoutParams valueParams = new LinearLayout.LayoutParams(-2, -2);
                valueParams.setMarginStart(dp(10));
                setting.addView(settingValue, valueParams);
                view = setting;
                break;
            default:
                TextView value = label("heading".equals(kind) ? 17 : 14);
                if ("heading".equals(kind)) {
                    value.setTypeface(null, Typeface.BOLD);
                    markHeading(value);
                }
                value.setTextIsSelectable("folder".equals(id) || "address".equals(id));
                if ("folder".equals(id)) {
                    value.setMaxLines(2);
                    value.setEllipsize(android.text.TextUtils.TruncateAt.END);
                    value.setTextSize(12);
                    value.setOnClickListener(v -> new android.app.AlertDialog.Builder(activity)
                        .setMessage(value.getText()).setPositiveButton(android.R.string.ok, null).show());
                }
                view = value;
        }
        view.setTag(kind + ":" + id);
        return view;
    }

    private void update(View view, JSONObject item) {
        if (view == header) return;
        String kind = item.optString("kind"), id = item.optString("id");
        if (view instanceof NativeMicrophoneUi.Key) {
            NativeMicrophoneUi.Key key = (NativeMicrophoneUi.Key) view;
            boolean iconOnly = item.optBoolean("iconOnly");
            key.setGravity(Gravity.TOP | Gravity.CENTER_HORIZONTAL);
            key.setIncludeFontPadding(false);
            key.setMinimumHeight(dp(iconOnly ? 52 : tabletWide ? 68 : compact ? 56 : wide ? 54 : 66));
            key.setPadding(0, iconOnly ? 0 : dp(6) + dp(22) + dp(5), 0, iconOnly ? 0 : dp(6));
            key.setTextSize(tabletWide ? 13 : 12);
            setText(key, iconOnly ? "" : item.optString("label"));
            key.setContentDescription(item.optString("label") + " · " + item.optString("hint"));
            key.setEnabled(item.optBoolean("enabled"));
            key.setAlpha(key.isEnabled() ? 1f : .44f);
            key.update(microphoneState, dark, tabletWide, iconOnly ? ink() : muted(), iconOnly);
            return;
        }
        if (!"row".equals(kind) && !"column".equals(kind) && !"strip".equals(kind)) {
            String state = dark + ":" + wide + ":" + tabletWide + ":" + compact + ":" + item;
            if (state.equals(appliedState.put(view, state))) return;
        }
        String label = item.optString("label", ""), detail = item.optString("detail", "");
        boolean enabled = item.optBoolean("enabled", true);
        if ("connection-chip".equals(id)) {
            updateConnectionChip((TextView) view, label, item.optString("hint"), detail, item.optString("status"));
            return;
        }
        if (!id.startsWith("nav-") && view.isEnabled() != enabled) view.setEnabled(enabled);
        if (view instanceof DPadView) {
            DPadView dpad = (DPadView) view;
            dpad.bind(item.optJSONArray("children"), enabled, item.optString("value"));
            dpad.setAlpha(enabled ? 1f : .44f);
            dpad.invalidate();
            return;
        }
        if (view instanceof MoreTile tile) {
            tile.update(label, item.optString("icon"), enabled, item.optBoolean("selected"));
            return;
        }
        switch (kind) {
            case "row":
            case "column":
                DividedLayout group = (DividedLayout) view;
                group.applyStyle();
                reconcile(group, item.optJSONArray("children"), "row".equals(kind));
                break;
            case "strip":
                reconcile(view instanceof KeyboardGrid grid ? grid : (LinearLayout) ((HorizontalScrollView) view).getChildAt(0), item.optJSONArray("children"), true);
                break;
            case "switch":
                CompoundButton toggle = (CompoundButton) view;
                setSupportingText(toggle, label, detail);
                toggle.setChecked(item.optBoolean("value"));
                toggle.setMinimumHeight(dp(detail.isEmpty() ? 52 : 64));
                int padding = dp(detail.isEmpty() ? 8 : 12);
                toggle.setPaddingRelative(dp(16), padding, dp(76), padding);
                toggle.setLineSpacing(dp(2), 1f);
                toggle.setTextColor(ink());
                toggle.setBackground(ripple(Color.TRANSPARENT));
                break;
            case "slider":
                LinearLayout slider = (LinearLayout) view;
                LinearLayout sliderTitle = (LinearLayout) slider.getChildAt(0);
                TextView caption = (TextView) sliderTitle.getChildAt(0), amount = (TextView) sliderTitle.getChildAt(1);
                double value = item.optDouble("value", 1.5);
                setText(caption, label);
                setText(amount, String.format(java.util.Locale.ROOT, "%.1f×", value));
                caption.setTextColor(ink());
                amount.setTextColor(accent());
                amount.setBackground(shape(accentSoft(), 8));
                SeekBar seek = (SeekBar) slider.getChildAt(1);
                seek.setContentDescription(label);
                seek.setProgressTintList(ColorStateList.valueOf(accent()));
                seek.setProgressBackgroundTintList(ColorStateList.valueOf(accentSoft()));
                seek.setThumbTintList(ColorStateList.valueOf(accent()));
                seek.setProgress((int) Math.round(value * 10 - 5));
                LinearLayout scale = (LinearLayout) slider.getChildAt(2);
                TextView low = (TextView) scale.getChildAt(0), high = (TextView) scale.getChildAt(1);
                setText(low, item.optString("hint"));
                setText(high, detail);
                low.setTextColor(muted());
                high.setTextColor(muted());
                break;
            case "edit":
                SessionEdit edit = (SessionEdit) view;
                edit.setTextColor(ink());
                edit.setHintTextColor(muted());
                edit.setHint(label);
                applyEditBackground(edit);
                edit.setOnFocusChangeListener((v, focused) -> applyEditBackground(edit));
                edit.setPadding(dp(12), dp(10), dp(12), dp(10));
                String draft = item.optString("value", "");
                int revision = item.optInt("revision");
                if (revision < edit.revision) break;
                edit.revision = revision;
                if (!draft.contentEquals(edit.getText())) {
                    editing = true;
                    edit.setText(draft);
                    edit.setSelection(edit.length());
                    editing = false;
                }
                break;
            case "progress":
                view.setContentDescription(label);
                ((ProgressBar) view).setProgress(item.optInt("value"));
                ((ProgressBar) view).setProgressTintList(ColorStateList.valueOf(accent()));
                break;
            case "window":
                LinearLayout windowRow = (LinearLayout) view;
                ImageView windowIcon = (ImageView) windowRow.getChildAt(0);
                LinearLayout windowCopy = (LinearLayout) windowRow.getChildAt(1);
                TextView windowTitle = (TextView) windowCopy.getChildAt(0);
                TextView windowApp = (TextView) windowCopy.getChildAt(1);
                TextView windowState = (TextView) windowRow.getChildAt(2);
                windowRow.setBackground(ripple(tonal(), 14));
                windowRow.setAlpha(enabled ? 1f : .5f);
                String status = item.optString("status");
                windowRow.setContentDescription(label + (detail.isEmpty() ? "" : " · " + detail) + (status.isEmpty() ? "" : " · " + status));
                windowTitle.setTextColor(enabled ? ink() : muted());
                windowTitle.setTextSize(14);
                windowTitle.setMaxLines(1);
                windowTitle.setEllipsize(TextUtils.TruncateAt.END);
                setText(windowTitle, label);
                windowApp.setTextColor(muted());
                windowApp.setTextSize(12);
                setText(windowApp, detail);
                windowApp.setVisibility(detail.isEmpty() ? View.GONE : View.VISIBLE);
                windowState.setTextColor(accent());
                windowState.setTextSize(12);
                setText(windowState, item.optString("status"));
                applyWindowIcon(windowIcon, item.optString("icon"));
                break;
            case "setting-button":
                LinearLayout setting = (LinearLayout) view;
                TextView settingLabel = (TextView) setting.getChildAt(0);
                TextView settingValue = (TextView) setting.getChildAt(1);
                settingLabel.setTextColor(enabled ? ink() : muted());
                settingValue.setTextColor(muted());
                settingValue.setCompoundDrawablePadding(dp(5));
                settingValue.setCompoundDrawablesRelative(NativeIcon.of(item.optString("icon"), accent(), dp(16)), null,
                    NativeIcon.of("next", muted(), dp(16)), null);
                setting.setBackground(ripple(Color.TRANSPARENT));
                setText(settingLabel, label);
                setText(settingValue, detail);
                setting.setContentDescription(detail.isEmpty() ? label : label + "，" + detail);
                break;
            default:
                TextView control = (TextView) view;
                control.setTextColor("heading".equals(kind) ? accent() : enabled || id.startsWith("nav-") ? ink() : muted());
                setSupportingText(control, label, detail);
                if ("button".equals(kind) || "pad".equals(kind) || "mic".equals(kind)) {
                    styleControl(control, id, label, detail, enabled, item.optString("icon"), item.optString("status"), item.has("selected") && "button".equals(kind), item.optBoolean("iconOnly"));
                    if (item.has("status") && !"pad".equals(kind))
                        control.setContentDescription(item.optString("status"));
                    if (item.has("selected")) {
                        control.setSelected(item.optBoolean("selected"));
                    }
                    if (control instanceof TouchPadView) {
                        TouchPadView pad = (TouchPadView) control;
                        pad.status = item.optString("status");
                        pad.hint = item.optString("hint");
                        String description = String.join(" · ", label, pad.status, detail);
                        pad.setContentDescription(pad.hint.isEmpty() ? description : description + " · " + pad.hint);
                    }
                } else {
                    control.setPadding(dp(8), dp(6), dp(8), dp(6));
                    if ("receive".equals(id) || "shared".equals(id)) {
                        control.setTextColor(ink());
                        control.setTextSize("shared".equals(id) ? 16 : 14);
                        control.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
                    }
                    if ("receive".equals(id) || "folder".equals(id)) control.setPadding(dp(8), 0, dp(8), 0);
                    if ("shared".equals(id)) control.setPadding(dp(16), dp(6), 0, dp(6));
                    if ("folder".equals(id) || id.startsWith("history-")) {
                        control.setTextSize(12);
                        control.setTextColor(muted());
                    }
                    if (id.startsWith("file-name-")) {
                        control.setPadding(dp(8), dp(8), 0, dp(8));
                        if (label.length() > 32) {
                            setSupportingText(control, label.replace("_", "_\u200b"), detail);
                            control.setContentDescription(label + ", " + detail);
                        }
                        control.setCompoundDrawablesRelative(NativeIcon.of("document", muted(), dp(22)), null, null, null);
                        control.setCompoundDrawablePadding(dp(12));
                        control.setTextSize(14);
                    }
                    if ("transfer-stage".equals(id)) {
                        control.setTextSize(15);
                        control.setTextColor(ink());
                    }
                    if ("transfer".equals(id)) {
                        control.setTextColor(muted());
                        control.setBackground(shape(tonal(), 12));
                        control.setPadding(dp(12), dp(8), dp(12), dp(8));
                    }
                    if (id.endsWith("-heading") || "display".equals(id) || "shortcuts".equals(id) || "computer-help".equals(id)) {
                        control.setTextSize(12);
                        control.setLetterSpacing(.04f);
                        control.setPadding(dp(16), dp(4), dp(16), dp(4));
                        if ("text-heading".equals(id)) control.setPadding(0, 0, 0, 0);
                    }
                    if ("shortcut-tip".equals(id) || "settings-footnote".equals(id)) {
                        control.setTextSize(12);
                        control.setTextColor(muted());
                        if ("settings-footnote".equals(id)) {
                            SpannableString footer = new SpannableString(label + " · " + detail);
                            URLSpan projectLink = new URLSpan("https://github.com/kuiwaiwai/plumelink") {
                                @Override
                                public void updateDrawState(TextPaint paint) {
                                    super.updateDrawState(paint);
                                    paint.setUnderlineText(false);
                                }
                            };
                            footer.setSpan(projectLink, 0, label.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
                            control.setText(footer);
                            control.setLinkTextColor(accent());
                            control.setMovementMethod(LinkMovementMethod.getInstance());
                            control.setGravity(Gravity.CENTER);
                            control.setMinimumHeight(dp(AdaptiveLayout.TOUCH_TARGET_DP));
                        }
                        else control.setPadding(dp(16), dp(5), dp(16), 0);
                    }
                    if ("draft-label".equals(id) || "clipboard".equals(id) || "character-count".equals(id)) {
                        control.setTextSize("draft-label".equals(id) ? 13 : 11);
                        control.setTextColor(muted());
                        control.setPadding(0, dp(3), 0, dp(3));
                    }
                    if ("pc-name".equals(id) || "pc-status".equals(id) || "address".equals(id)) {
                        control.setGravity(Gravity.CENTER);
                        control.setTextColor("pc-name".equals(id) ? ink() : muted());
                        control.setPadding(dp(10), dp(4), dp(10), dp(4));
                        if ("pc-name".equals(id)) {
                            control.setTextSize(18);
                            control.setCompoundDrawables(null, NativeIcon.of("monitor", accent(), dp(40)), null, null);
                            control.setCompoundDrawablePadding(dp(12));
                        } else control.setTextSize(13);
                    }
                    if ("quality".equals(id) || "permission".equals(id)) {
                        control.setPadding(dp(16), dp(12), dp(16), dp(12));
                        control.setMinimumHeight(dp(60));
                        control.setGravity(Gravity.CENTER_VERTICAL);
                        control.setTextColor(ink());
                    }
                    if ("connection-latency".equals(id)) {
                        control.setPadding(dp(4), 0, dp(16), 0);
                        control.setTextSize(14);
                    }
                    if ("transport".equals(id) || "permissions".equals(id)) {
                        control.setTextColor(muted());
                        control.setTextSize(13);
                        control.setPadding(dp(10), dp(3), dp(10), dp(3));
                    }
                    if ("files-empty".equals(id)) {
                        control.setGravity(Gravity.CENTER);
                        control.setPadding(dp(18), dp(18), dp(18), dp(20));
                        control.setCompoundDrawables(null, NativeIcon.of("folder", muted(), dp(26)), null, null);
                        control.setCompoundDrawablePadding(dp(8));
                    }
                    if ("sheet-feedback".equals(id)) {
                        control.setTextColor(accent());
                        control.setPadding(dp(12), dp(10), dp(12), dp(10));
                        control.setBackground(shape(accentSoft(), 12));
                    }
                    if ("sheet-feedback".equals(id) || "switch-help".equals(id)) {
                        control.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
                    }
                }
        }
    }

    private void applyWindowIcon(ImageView view, String source) {
        if (source.startsWith("data:image/")) {
            String key = "bitmap:" + source;
            if (key.equals(view.getTag())) return;
            int comma = source.indexOf(',');
            if (comma > 0) {
                try {
                    byte[] bytes = Base64.decode(source.substring(comma + 1), Base64.DEFAULT);
                    Bitmap bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
                    if (bitmap != null) {
                        view.setImageBitmap(bitmap);
                        view.setTag(key);
                        return;
                    }
                } catch (IllegalArgumentException ignored) { /* Fall through to the placeholder. */ }
            }
        }
        if (!"fallback".equals(view.getTag())) {
            view.setImageDrawable(NativeIcon.of("monitor", muted(), dp(28)));
            view.setTag("fallback");
        }
    }

    private void applyEditBackground(EditText edit) {
        GradientDrawable field = shape(tonal(), 14);
        if (edit.hasFocus()) field.setStroke(dp(1), accent());
        edit.setBackground(field);
    }

    private void constrainToLines(TextView text, int lines) {
        text.setMaxLines(lines);
        text.setEllipsize(TextUtils.TruncateAt.END);
    }

    private final class SettingRow extends LinearLayout {
        private boolean stacked;
        SettingRow() { super(activity); }
        @Override public CharSequence getAccessibilityClassName() { return Button.class.getName(); }
        @Override protected void onMeasure(int widthMeasureSpec, int heightMeasureSpec) {
            TextView caption = (TextView) getChildAt(0);
            TextView value = (TextView) getChildAt(1);
            int inner = MeasureSpec.getSize(widthMeasureSpec) - getPaddingLeft() - getPaddingRight();
            boolean stack = value.length() > 0 && MeasureSpec.getMode(widthMeasureSpec) != MeasureSpec.UNSPECIFIED
                && !AdaptiveLayout.fitsInline(inner, desiredWidth(caption), desiredWidth(value), dp(10));
            if (stacked != stack) {
                stacked = stack;
                setOrientation(stack ? VERTICAL : HORIZONTAL);
                LinearLayout.LayoutParams captionParams = (LinearLayout.LayoutParams) caption.getLayoutParams();
                captionParams.width = stack ? LayoutParams.MATCH_PARENT : 0;
                captionParams.weight = stack ? 0 : 1;
                LinearLayout.LayoutParams valueParams = (LinearLayout.LayoutParams) value.getLayoutParams();
                valueParams.width = stack ? LayoutParams.MATCH_PARENT : LayoutParams.WRAP_CONTENT;
                valueParams.setMarginStart(stack ? 0 : dp(10));
                valueParams.topMargin = stack ? dp(2) : 0;
                value.setGravity(stack ? Gravity.START : Gravity.END);
            }
            super.onMeasure(widthMeasureSpec, heightMeasureSpec);
        }
    }

    private static float desiredWidth(TextView text) {
        return Layout.getDesiredWidth(text.getText(), text.getPaint())
            + text.getCompoundPaddingLeft() + text.getCompoundPaddingRight();
    }

    private void updateConnectionChip(TextView chip, String accessible, String label, String latency, String status) {
        String style = dark + ":" + status;
        boolean styleChanged = !style.equals(header.chipStyle);
        if (styleChanged) {
            header.chipStyle = style;
            styleControl(chip, "connection-chip", label, "", true, "", status, false, false);
            GradientDrawable dot = shape(chip.getCurrentTextColor(), 8);
            dot.setBounds(0, 0, dp(6), dp(6));
            chip.setCompoundDrawablesRelative(dot, null, NativeIcon.of("next", chip.getCurrentTextColor(), dp(16)), null);
            chip.setCompoundDrawablePadding(dp(6));
            header.chipTextDirty = true;
        }
        if (!TextUtils.equals(chip.getContentDescription(), accessible)) chip.setContentDescription(accessible);
        String value = latency.isEmpty() ? label : label + "  " + latency;
        if (!styleChanged && label.equals(header.connectionLabel) && TextUtils.equals(value, header.connectionWithLatency)) return;
        SpannableString styled = new SpannableString(value);
        if (!latency.isEmpty()) {
            styled.setSpan(new ForegroundColorSpan(muted()), value.length() - latency.length(), value.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            styled.setSpan(new RelativeSizeSpan(.83f), value.length() - latency.length(), value.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        }
        header.connectionLabel = label;
        header.connectionWithLatency = styled;
        chip.requestLayout();
    }

    private void styleControl(TextView view, String id, String label, String detail, boolean enabled, String explicitIcon, String status, boolean option, boolean iconOnly) {
        if (view instanceof TouchPadView) {
            TouchPadView pad = (TouchPadView) view;
            pad.caption = label;
            setText(pad, detail);
            pad.setTextColor(muted());
            pad.invalidate();
            return;
        }
        boolean nav = id.startsWith("nav-");
        boolean selected = nav && !enabled;
        String group = view.getParent() instanceof View parent ? String.valueOf(parent.getTag()) : "";
        boolean rocker = group.startsWith("row:rocker-");
        boolean media = rocker || group.equals("row:volume") || group.equals("row:playback");
        boolean tools = group.startsWith("row:tools");
        boolean mouse = group.equals("row:clicks");
        boolean settings = group.startsWith("column:settings-");
        boolean fileRow = id.startsWith("file-");
        boolean saveFile = id.startsWith("save-");
        boolean history = "column:file-history-card".equals(group);
        boolean folderAction = "pick-folder".equals(id) || "reload".equals(id) || id.startsWith("save-");
        boolean primary = "send".equals(id) || "pair".equals(id) || "upload".equals(id) || "window-confirm".equals(id)
            || "reconnect".equals(id);
        boolean connection = "connection-chip".equals(id);
        boolean key = id.startsWith("key-"), combo = id.startsWith("combo-");
        boolean session = group.equals("row:session");
        boolean danger = "power".equals(id);
        boolean forget = "forget".equals(id);
        boolean diagnostics = "diagnostics".equals(id);
        if (key || combo) constrainToLines(view, 1);
        else {
            view.setMaxLines(Integer.MAX_VALUE);
            view.setEllipsize(null);
        }
        if (!connection && !media) view.setContentDescription(detail.isEmpty() ? label : label + "，" + detail);
        if (folderAction) {
            view.setMinimumWidth(dp(AdaptiveLayout.TOUCH_TARGET_DP));
        }
        int connectionInk = "fair".equals(status) ? (dark ? 0xffffd18b : 0xff925b00)
            : "poor".equals(status) ? (dark ? 0xffffb1b6 : 0xffb73b45)
            : "good".equals(status) || "unknown".equals(status) ? (dark ? 0xff6bd5ae : 0xff0f7555) : muted();
        int foreground = primary ? (dark ? 0xff122448 : Color.WHITE) : selected || combo ? accent() : enabled ? ink() : muted();
        int fill = primary ? accent() : key || "change".equals(id) || group.startsWith("row:window-") ? tonal() : combo ? accentSoft()
            : connection ? ("good".equals(status) ? (dark ? 0xff233b36 : 0xffe7f1ef) : tonal())
            : danger ? (dark ? 0xff362127 : 0xfffff0f1)
            : saveFile && enabled ? accentSoft()
            : option || forget || diagnostics || history ? Color.TRANSPARENT
            : nav || media || tools && !tabletWide || mouse || settings || session || fileRow || folderAction ? Color.TRANSPARENT : surface();
        view.setTextColor(connection ? connectionInk : danger || forget ? (dark ? 0xffffb1b6 : 0xffb73b45)
            : diagnostics ? muted() : folderAction ? accent() : foreground);
        view.setBackground(session && !"focus".equals(id) ? new InsetDrawable(ripple(tonal(), 12), 0, dp(5), 0, dp(5))
            : ripple(fill, connection || primary && !media ? 24 : option ? 12 : 16));
        view.setAlpha(!enabled && !nav ? .44f : 1f);
        view.setGravity(settings || fileRow || option || diagnostics || "history-toggle".equals(id)
            ? Gravity.START | Gravity.CENTER_VERTICAL : Gravity.CENTER);
        view.setTextSize(nav ? 11 : tools && tabletWide ? 13 : session || connection || tools ? 12 : diagnostics ? 13 : 14);
        view.setMinimumHeight(dp(nav ? (compact ? 56 : 62) : tools ? (tabletWide ? 68 : compact ? 56 : wide ? 54 : 66)
            : mouse ? 52 : option ? (detail.isEmpty() ? 48 : 60) : settings || fileRow ? 60 : "upload".equals(id) ? 50 : 48));
        int horizontalPadding = session || folderAction || history ? 8 : settings || connection || fileRow ? 12 : 4;
        view.setPadding(dp(horizontalPadding), dp(6), dp(horizontalPadding), dp(6));
        view.setCompoundDrawablePadding(dp(5));
        if (key || combo) {
            view.setMinimumWidth(dp(key ? 50 : 56));
            view.setPadding(dp(13), dp(6), dp(13), dp(6));
        }
        if (option) view.setPadding(dp(4), dp(8), dp(44), dp(8));
        if (diagnostics) view.setPadding(dp(8), dp(8), dp(10), dp(8));
        if (view instanceof ControlButton) ((ControlButton) view).option = option;
        if (tools && view instanceof ControlButton button) {
            button.centerStack = true;
            button.setGravity(Gravity.TOP | Gravity.CENTER_HORIZONTAL);
            button.setIncludeFontPadding(false);
        }
        String icon = explicitIcon.isEmpty() ? iconFor(id) : explicitIcon;
        if (icon != null) {
            int color = primary || selected ? foreground : media ? ink() : muted();
            Drawable glyph = NativeIcon.of(icon, option ? accent() : color, dp(diagnostics ? 16 : session ? 18 : 22));
            if (nav && glyph != null) {
                LayerDrawable pill = new LayerDrawable(new Drawable[]{shape(selected ? accentSoft() : Color.TRANSPARENT, 20), glyph});
                pill.setLayerGravity(1, Gravity.CENTER);
                pill.setLayerSize(1, dp(22), dp(22));
                pill.setBounds(0, 0, dp(wide ? 54 : 64), dp(32));
                glyph = pill;
            }
            if ("upload".equals(id)) {
                view.setBackground(ripple(accentSoft(), 18));
                view.setTextColor(ink());
                view.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
                view.setMinimumHeight(dp(70));
                view.setPadding(dp(16), dp(12), dp(16), dp(12));
                view.setCompoundDrawablePadding(dp(12));
                view.setCompoundDrawablesRelative(NativeIcon.of("upload", accent(), dp(24)), null, NativeIcon.of("next", muted(), dp(16)), null);
            } else if ("send".equals(id)) {
                ((ControlButton) view).inlineIcon = glyph;
                view.setCompoundDrawables(null, null, null, null);
            } else if ("history-toggle".equals(id)) view.setCompoundDrawablesRelative(null, null, glyph, null);
            else if (tools || nav) view.setCompoundDrawables(null, glyph, null, null);
            else view.setCompoundDrawablesRelative(glyph, null, null, null);
        } else view.setCompoundDrawables(null, null, null, null);
        boolean padLayoutAction = session && view.getParent().getParent() instanceof View pane
            && "column:tool-column".equals(pane.getTag()) && ("focus".equals(id) || "pad-side".equals(id));
        if (iconOnly || padLayoutAction) {
            setText(view, "");
            view.setCompoundDrawables(null, null, null, null);
            view.setGravity(Gravity.CENTER);
            view.setPadding(0, 0, 0, 0);
            int target = dp(padLayoutAction ? 48 : 52);
            view.setMinWidth(target);
            view.setMinimumWidth(target);
            view.setMinHeight(target);
            view.setMinimumHeight(target);
            view.setBackground(ripple(padLayoutAction ? Color.TRANSPARENT : primary ? accent() : group.startsWith("row:window-actions") ? Color.TRANSPARENT : tonal(), "row:handset-tools".equals(group) ? 20 : 26));
            if (view instanceof ControlButton) {
                ControlButton button = (ControlButton) view;
                button.inlineIcon = null;
                button.centerIcon = icon == null ? null
                    : NativeIcon.of(icon, padLayoutAction ? muted() : primary ? (dark ? 0xff122448 : Color.WHITE) : enabled ? ink() : muted(), dp(padLayoutAction ? 20 : 24));
            }
            if (padLayoutAction) {
                view.setContentDescription(label);
                view.setTooltipText(label);
            } else if (TextUtils.isEmpty(view.getContentDescription())) view.setContentDescription(label);
            return;
        }
        if (media || saveFile || "reload".equals(id)) {
            view.setCompoundDrawables(null, null, null, null);
            ((ControlButton) view).centerIcon = NativeIcon.of(icon, media ? foreground : enabled ? accent() : muted(), dp(rocker ? 24 : 21));
            view.setContentDescription(label);
            setText(view, "");
        }
        if (nav) {
            view.setSelected(selected);
            view.setEnabled(true);
        }
    }

    private void drawCheck(Canvas canvas, Paint paint, float x, float y, int color) {
        paint.setColor(color);
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(dp(2));
        paint.setStrokeCap(Paint.Cap.ROUND);
        canvas.drawLine(x - dp(4), y, x - dp(1), y + dp(3), paint);
        canvas.drawLine(x - dp(1), y + dp(3), x + dp(5), y - dp(3), paint);
    }

    private final class PreferenceToggle extends CompoundButton {
        private final Paint toggle = new Paint(Paint.ANTI_ALIAS_FLAG);
        PreferenceToggle() {
            super(activity);
            setButtonDrawable((Drawable) null);
            setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
            setTextSize(14);
        }
        @Override public CharSequence getAccessibilityClassName() { return Switch.class.getName(); }
        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            boolean rtl = getLayoutDirection() == View.LAYOUT_DIRECTION_RTL;
            float left = rtl ? dp(16) : getWidth() - dp(64), y = getHeight() * .5f;
            int off = dark ? 0xff5a6474 : 0xffadb6c6;
            toggle.setStyle(Paint.Style.FILL);
            toggle.setColor(isChecked() ? accent() : tonal());
            canvas.drawRoundRect(left, y - dp(14), left + dp(48), y + dp(14), dp(14), dp(14), toggle);
            if (!isChecked()) {
                toggle.setStyle(Paint.Style.STROKE);
                toggle.setStrokeWidth(dp(2));
                toggle.setColor(off);
                canvas.drawRoundRect(left + dp(1), y - dp(13), left + dp(47), y + dp(13), dp(13), dp(13), toggle);
            }
            float x = left + dp(isChecked() != rtl ? 34 : 14);
            toggle.setStyle(Paint.Style.FILL);
            toggle.setColor(isChecked() ? (dark ? 0xff122448 : Color.WHITE) : off);
            canvas.drawCircle(x, y, dp(isChecked() ? 11 : 8), toggle);
            if (isChecked()) drawCheck(canvas, toggle, x, y, accent());
        }
    }

    private final class DividedLayout extends LinearLayout {
        private final String id;
        private final Paint divider = new Paint();
        private Boolean styledDark;
        DividedLayout(String id) {
            super(activity);
            this.id = id;
            if ("landscape".equals(id)) {
                GradientDrawable gap = shape(Color.TRANSPARENT, 0);
                gap.setSize(dp(16), 1);
                setDividerDrawable(gap);
                setShowDividers(SHOW_DIVIDER_MIDDLE);
            }
        }

        @Override protected void onMeasure(int width, int height) {
            if ("handset-portrait".equals(id) || "keyboard-column".equals(id)) {
                int limit = dp("keyboard-column".equals(id) ? 960 : 368);
                if (MeasureSpec.getSize(width) > limit) width = MeasureSpec.makeMeasureSpec(limit, MeasureSpec.EXACTLY);
            }
            if ("keyboard-column".equals(id) && getChildCount() == 2) {
                boolean split = !shortKeyboard && MeasureSpec.getSize(width) >= dp(640);
                getChildAt(1).setVisibility(shortKeyboard ? GONE : VISIBLE);
                setOrientation(split ? HORIZONTAL : VERTICAL);
                setGravity(Gravity.TOP);
                for (int i = 0; i < 2; i++) {
                    LayoutParams pane = (LayoutParams) getChildAt(i).getLayoutParams();
                    pane.width = split ? 0 : LayoutParams.MATCH_PARENT;
                    pane.height = LayoutParams.WRAP_CONTENT;
                    pane.weight = split ? 1 : 0;
                    pane.setMargins(i == 1 && split ? dp(20) : 0, i == 1 && !split ? dp(14) : 0, 0, 0);
                }
            }
            if ("keyboard-entry".equals(id) && getChildCount() == 2) {
                setOrientation(shortKeyboard ? HORIZONTAL : VERTICAL);
                setGravity(Gravity.CENTER_VERTICAL);
                LayoutParams editor = (LayoutParams) getChildAt(0).getLayoutParams();
                editor.width = shortKeyboard ? 0 : LayoutParams.MATCH_PARENT;
                editor.weight = shortKeyboard ? 1 : 0;
                LayoutParams footer = (LayoutParams) getChildAt(1).getLayoutParams();
                footer.width = shortKeyboard ? dp(52) : LayoutParams.MATCH_PARENT;
                footer.setMargins(shortKeyboard ? dp(8) : 0, shortKeyboard ? 0 : dp(4), 0, 0);
                LinearLayout meta = (LinearLayout) getChildAt(1);
                for (int i = 0; i < meta.getChildCount() - 1; i++)
                    meta.getChildAt(i).setVisibility(shortKeyboard ? GONE : VISIBLE);
                LayoutParams send = (LayoutParams) meta.getChildAt(meta.getChildCount() - 1).getLayoutParams();
                send.leftMargin = shortKeyboard ? 0 : dp(8);
            }
            if ("handset-portrait".equals(id) && getChildCount() > 0) {
                int dialHeight = dp(((DPadView) getChildAt(0)).diameterDp + 8);
                if (MeasureSpec.getMode(height) != MeasureSpec.UNSPECIFIED) {
                    int available = MeasureSpec.getSize(height) - getPaddingTop() - getPaddingBottom();
                    for (int i = 1; i < getChildCount(); i++) {
                        LayoutParams row = (LayoutParams) getChildAt(i).getLayoutParams();
                        available -= row.height + row.topMargin + row.bottomMargin;
                    }
                    if (MeasureSpec.getMode(height) == MeasureSpec.EXACTLY) dialHeight = available;
                    dialHeight = Math.max(dp(DIAL_MIN_HEIGHT_DP), Math.min(dialHeight, available));
                }
                getChildAt(0).getLayoutParams().height = dialHeight;
            }
            if (getChildCount() == 2 && MeasureSpec.getMode(width) != MeasureSpec.UNSPECIFIED
                && ("folder-card".equals(id) || "files-header".equals(id))) {
                View leading = getChildAt(0);
                TextView title = (TextView) (leading instanceof LinearLayout column ? column.getChildAt(0) : leading);
                TextView action = (TextView) getChildAt(1);
                int inner = MeasureSpec.getSize(width) - getPaddingLeft() - getPaddingRight();
                float actionWidth = Math.max(action.getMinimumWidth(), desiredWidth(action));
                int gap = dp(8);
                boolean inline = AdaptiveLayout.fitsInline(inner, desiredWidth(title), actionWidth, gap);
                setOrientation(inline ? HORIZONTAL : VERTICAL);
                LayoutParams caption = (LayoutParams) leading.getLayoutParams();
                caption.width = inline ? 0 : LayoutParams.MATCH_PARENT;
                caption.weight = inline ? 1 : 0;
                LayoutParams trailing = (LayoutParams) action.getLayoutParams();
                trailing.width = LayoutParams.WRAP_CONTENT;
                trailing.gravity = inline ? Gravity.CENTER_VERTICAL : Gravity.START;
                trailing.setMarginStart(inline ? gap : 0);
                trailing.topMargin = inline ? 0 : dp(4);
            }
            super.onMeasure(width, height);
        }

        void applyStyle() {
            if (styledDark != null && styledDark == dark) return;
            styledDark = dark;
            if (id.startsWith("more-double-")) setWeightSum(2);
            if (id.startsWith("more-icons-")) setWeightSum(Integer.parseInt(id.split("-")[2]));
            if ("handset-portrait".equals(id) || "tool-column".equals(id)) setGravity(Gravity.CENTER);
            if ("handset-portrait".equals(id)) setPadding(0, 0, 0, dp(20));
            if ("session".equals(id)) setGravity(Gravity.END | Gravity.CENTER_VERTICAL);
            if (id.startsWith("file-") && getOrientation() == HORIZONTAL) {
                setGravity(Gravity.CENTER_VERTICAL);
                setMinimumHeight(dp(66));
            }
            if ("folder-card".equals(id) || "files-header".equals(id) || "quality-row".equals(id)) setGravity(Gravity.CENTER_VERTICAL);
            if ("folder-card".equals(id)) setPadding(dp(8), dp(8), dp(8), dp(8));
            if ("files-header".equals(id)) setPadding(0, 0, dp(8), 0);
            if ("file-library-card".equals(id) || "file-history-card".equals(id)) {
                setPadding(dp(8), dp(4), dp(8), dp(4));
            }
            if ("transfer-card".equals(id)) setPadding(dp(8), dp(8), dp(8), dp(8));
            if ("connection-hero".equals(id)) setPadding(0, dp(10), 0, dp(8));
            if ("pad-column".equals(id)) {
                GradientDrawable border = shape(surface(), 24);
                border.setStroke(dp(1), edge());
                setBackground(border);
                setClipToOutline(true);
            } else if (id.startsWith("rocker-") || id.equals("tools") || "volume".equals(id) || "playback".equals(id) || id.startsWith("settings-")
                || "folder-card".equals(id) || "file-library-card".equals(id) || "file-history-card".equals(id) || "transfer-card".equals(id)
                || "connection-metrics".equals(id)) {
                setBackground(shape("connection-metrics".equals(id) || "tools".equals(id) ? tonal() : surface(), 18));
                setClipToOutline(true);
            }
        }
        @Override protected void dispatchDraw(Canvas canvas) {
            super.dispatchDraw(canvas);
            divider.setColor(edge());
            divider.setStrokeWidth(dp(1));
            boolean clicks = "clicks".equals(id);
            if (clicks || id.startsWith("rocker-")) {
                if (clicks) canvas.drawLine(0, .5f, getWidth(), .5f, divider);
                float centerY = getHeight() / 2f;
                for (int i = 1; i < getChildCount(); i++) {
                    float x = getChildAt(i).getLeft() - (clicks ? dp(3) : 0);
                    canvas.drawLine(x, centerY - dp(11), x, centerY + dp(11), divider);
                }
            } else if (id.startsWith("settings-") || "file-library-card".equals(id) || "file-history-card".equals(id) || "connection-metrics".equals(id)) {
                for (int i = 1; i < getChildCount(); i++) {
                    float y = getChildAt(i).getTop();
                    int inset = "file-library-card".equals(id) || "file-history-card".equals(id) ? getPaddingLeft()
                        : id.startsWith("settings-") && (!"settings-shortcuts".equals(id) || i % 2 == 1) ? dp(16) : 0;
                    canvas.drawLine(inset, y, getWidth() - inset, y, divider);
                }
            }
        }
    }

    private final class MoreTile extends LinearLayout {
        private final String id;
        private final ImageView glyph;
        private final TextView caption;

        MoreTile(String id) {
            super(activity);
            this.id = id;
            setOrientation(VERTICAL);
            setGravity(Gravity.CENTER);
            setMinimumHeight(dp(compactMoreSheet ? 52 : 80));
            int verticalPadding = dp(compactMoreSheet ? 4 : 8);
            setPadding(dp(MORE_TILE_PADDING_DP), verticalPadding, dp(MORE_TILE_PADDING_DP), verticalPadding);
            setOnClickListener(v -> emit(id, null));
            setFocusable(true);

            glyph = new ImageView(activity);
            glyph.setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
            addView(glyph, new LinearLayout.LayoutParams(dp(20), dp(20)));

            caption = label(MORE_TILE_TEXT_SP);
            caption.setGravity(Gravity.CENTER);
            caption.setIncludeFontPadding(false);
            caption.setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
            LinearLayout.LayoutParams captionLayout = new LinearLayout.LayoutParams(-1, -2);
            captionLayout.topMargin = dp(compactMoreSheet ? 3 : 8);
            addView(caption, captionLayout);
        }

        void update(String label, String icon, boolean enabled, boolean selected) {
            setSelected(selected);
            if (selected) {
                GradientDrawable outline = shape(accentSoft(), 16);
                outline.setStroke(dp(2), accent());
                setBackground(new RippleDrawable(ColorStateList.valueOf(edge()), outline, null));
            } else setBackground(ripple(tonal()));
            setAlpha(enabled ? 1f : .44f);
            setContentDescription(label);
            glyph.setImageDrawable(NativeIcon.of(icon.isEmpty() ? iconFor(id) : icon, accent(), dp(20)));
            caption.setTextColor(ink());
            setText(caption, label);
        }

        @Override public CharSequence getAccessibilityClassName() {
            return Button.class.getName();
        }
    }

    // CENTER gravity leaves a top compound drawable at the padding edge.
    static int centeredStackOffset(TextView view) {
        if (view.getLayout() == null) return 0;
        int height = view.getCompoundPaddingTop() + view.getLayout().getHeight() + view.getCompoundPaddingBottom();
        return Math.max(0, (view.getHeight() - height) / 2);
    }

    private final class ControlButton extends Button {
        boolean centerStack;
        Drawable centerIcon;
        Drawable inlineIcon;
        boolean option;
        final Paint radio = new Paint(Paint.ANTI_ALIAS_FLAG);
        ControlButton() { super(activity); }
        @Override public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info) {
            super.onInitializeAccessibilityNodeInfo(info);
            if (option) {
                info.setClassName(RadioButton.class.getName());
                info.setCheckable(true);
                info.setChecked(isSelected());
                info.setSelected(false);
            }
        }
        @Override protected void onDraw(Canvas canvas) {
            if (centerStack) {
                canvas.save();
                canvas.translate(0, centeredStackOffset(this));
                super.onDraw(canvas);
                canvas.restore();
            } else if (inlineIcon != null && getLineCount() == 1) {
                float shift = (inlineIcon.getBounds().width() + dp(6)) / 2f;
                canvas.save();
                canvas.translate(shift, 0);
                super.onDraw(canvas);
                canvas.restore();
                float start = getWidth() / 2f - getLayout().getLineWidth(0) / 2f - shift;
                drawIcon(canvas, inlineIcon, start);
            } else super.onDraw(canvas);
            if (centerIcon != null) drawIcon(canvas, centerIcon, (getWidth() - centerIcon.getBounds().width()) / 2f);
            if (option) {
                float x = getWidth() - dp(20), y = getHeight() / 2f;
                radio.setColor(isSelected() ? accent() : muted());
                radio.setStyle(isSelected() ? Paint.Style.FILL : Paint.Style.STROKE);
                radio.setStrokeWidth(dp(2));
                canvas.drawCircle(x, y, dp(10), radio);
                if (isSelected()) drawCheck(canvas, radio, x, y, dark ? 0xff122448 : Color.WHITE);
            }
        }
        private void drawIcon(Canvas canvas, Drawable icon, float start) {
            canvas.save();
            canvas.translate(start, (getHeight() - icon.getBounds().height()) / 2f);
            icon.draw(canvas);
            canvas.restore();
        }
    }

    private final class DPadView extends FrameLayout {
        private static final float CENTER_RATIO = .32f;
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path ring = new Path();
        private final Path shape = new Path();
        private final Path sectorPath = new Path();
        private final RectF outerRect = new RectF();
        private int pressed = -1;
        private int origin = -1;
        private float cx, cy, radius, innerRadius, centerRadius;

        DPadView() {
            super(activity);
            setWillNotDraw(false);
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
            setMinimumHeight(dp(DIAL_MIN_HEIGHT_DP));
            for (int i = 0; i < 5; i++) {
                Button key = new Button(activity);
                key.setStateListAnimator(null);
                key.setBackground(null);
                key.setOnClickListener(v -> emit((String) v.getTag(), null));
                key.setOnFocusChangeListener((v, focused) -> invalidate());
                addView(key, new FrameLayout.LayoutParams(dp(48), dp(48)));
            }
        }

        private int diameterDp = STANDARD_DIAL_DP;

        void bind(JSONArray keys, boolean enabled, String size) {
            int diameter = "small".equals(size) ? 176 : "large".equals(size) ? 256 : STANDARD_DIAL_DP;
            if (diameterDp != diameter) {
                diameterDp = diameter;
                requestLayout();
            }
            for (int i = 0; i < getChildCount(); i++) {
                JSONObject item = keys.optJSONObject(i);
                View key = getChildAt(i);
                key.setTag(item.optString("id"));
                key.setContentDescription(item.optString("label"));
                key.setEnabled(enabled);
            }
            if (!enabled) pressed = origin = -1;
        }

        @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
            super.onLayout(changed, l, t, r, b);
            updateGeometry(r - l, b - t);
            float mid = (radius + innerRadius) / 2f;
            for (int i = 0; i < getChildCount(); i++) {
                int x = Math.round(cx + (i == 1 ? mid : i == 3 ? -mid : 0));
                int y = Math.round(cy + (i == 2 ? mid : i == 0 ? -mid : 0));
                int half = dp(24);
                getChildAt(i).layout(x - half, y - half, x + half, y + half);
            }
        }

        private void updateGeometry(int w, int h) {
            int available = Math.max(0, Math.min(w, h) - dp(8));
            if (wide) {
                int availableDp = (int) (available / activity.getResources().getDisplayMetrics().density);
                radius = dp(AdaptiveLayout.dialDiameterDp(availableDp, diameterDp)) / 2f;
            } else radius = Math.min(dp(diameterDp), available) / 2f;
            cx = w / 2f;
            cy = h / 2f;
            if ("column:handset-portrait".equals(((View) getParent()).getTag())) {
                // Bias the dial toward the keys on tall handsets, keeping the drawing and hit areas together.
                float spaceAroundDial = Math.max(0, cy - radius - dp(4));
                cy += Math.min(dp(56), spaceAroundDial * .45f);
            }
            centerRadius = Math.max(dp(24), radius * CENTER_RATIO);
            innerRadius = centerRadius + dp(5);
            outerRect.set(cx - radius, cy - radius, cx + radius, cy + radius);
            ring.reset();
            ring.setFillType(Path.FillType.EVEN_ODD);
            ring.addCircle(cx, cy, radius, Path.Direction.CW);
            ring.addCircle(cx, cy, innerRadius, Path.Direction.CCW);
        }

        @Override protected void onDraw(Canvas canvas) {
            if (radius <= 0) return;
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(surface());
            canvas.drawPath(ring, paint);
            int active = pressed;
            if (active < 0 && getFocusedChild() != null) active = indexOfChild(getFocusedChild());
            if (active >= 0 && active < 4) {
                canvas.save();
                canvas.clipPath(sector(active));
                paint.setColor(accentSoft());
                canvas.drawPath(ring, paint);
                canvas.restore();
            }
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(dp(1));
            paint.setColor(edge());
            canvas.drawPath(ring, paint);
            for (int i = 0; i < 4; i++) drawArrow(canvas, i, active == i ? accent() : ink());
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(active == 4 ? accent() : accentSoft());
            canvas.drawCircle(cx, cy, centerRadius, paint);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeCap(Paint.Cap.ROUND);
            paint.setStrokeJoin(Paint.Join.ROUND);
            paint.setStrokeWidth(dp(2.5f));
            paint.setColor(active == 4 ? (dark ? 0xff122448 : Color.WHITE) : accent());
            float s = dp(9);
            shape.reset();
            shape.moveTo(cx - s, cy);
            shape.lineTo(cx - s * .18f, cy + s * .66f);
            shape.lineTo(cx + s * .92f, cy - s * .58f);
            canvas.drawPath(shape, paint);
        }

        private Path sector(int index) {
            sectorPath.reset();
            sectorPath.moveTo(cx, cy);
            sectorPath.arcTo(outerRect, -135f + index * 90f, 90f, false);
            sectorPath.close();
            return sectorPath;
        }

        private void drawArrow(Canvas canvas, int index, int color) {
            float mid = (radius + innerRadius) / 2f;
            float s = Math.min(dp(8), radius * .08f);
            canvas.save();
            canvas.rotate(index * 90f, cx, cy);
            shape.reset();
            shape.moveTo(cx - s, cy - mid + s * .4f);
            shape.lineTo(cx, cy - mid - s * .4f);
            shape.lineTo(cx + s, cy - mid + s * .4f);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeCap(Paint.Cap.ROUND);
            paint.setStrokeJoin(Paint.Join.ROUND);
            paint.setStrokeWidth(dp(2.5f));
            paint.setColor(color);
            canvas.drawPath(shape, paint);
            canvas.restore();
        }

        private int zone(float x, float y) {
            float dx = x - cx, dy = y - cy;
            float distance = (float) Math.hypot(dx, dy);
            if (distance <= centerRadius + dp(3)) return 4;
            if (distance > radius) return -1;
            int step = Math.round((float) Math.toDegrees(Math.atan2(dy, dx)) / 90f);
            if (step == 0) return 1; // right
            if (step == 1) return 2; // down
            if (step == -1) return 0; // up
            return 3; // left
        }

        // Keyboard and accessibility activation use the child buttons' click handlers.
        @Override public boolean onInterceptTouchEvent(MotionEvent event) { return true; }

        @Override public boolean onTouchEvent(MotionEvent event) {
            if (!isEnabled()) return false;
            int target = zone(event.getX(), event.getY());
            switch (event.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    if (target < 0) return false;
                    getParent().requestDisallowInterceptTouchEvent(true);
                    pressed = origin = target;
                    break;
                case MotionEvent.ACTION_MOVE:
                    pressed = target == origin ? origin : -1;
                    break;
                case MotionEvent.ACTION_UP:
                    if (target == origin && origin >= 0) getChildAt(origin).performClick();
                    // fall through to release the highlight
                case MotionEvent.ACTION_CANCEL:
                case MotionEvent.ACTION_POINTER_DOWN:
                    pressed = origin = -1;
                    break;
            }
            invalidate();
            return true;
        }
    }

    private String iconFor(String id) {
        if (id.startsWith("save-")) return "download";
        switch (id) {
            case "nav-remote": return "pointer";
            case "nav-files": return "files";
            case "nav-settings": return "settings";
            case "pad-side": return "side";
            case "offline-change": return "monitor";
            case "scroll-up": return "up";
            case "scroll-down": return "down";
            case "pick-folder": return "folder";
            case "reload": case "window-refresh": return "refresh";
            case "dpad-esc": return "back";
            case "esc": return "escape";
            case "taskview": return "taskview";
            case "enter": case "keyboard": case "mic": case "more":
            case "quieter": case "mute": case "louder": case "prev": case "play": case "next":
            case "focus": case "desktop": case "backspace": case "windows": case "send": case "upload":
                return id;
            default: return null;
        }
    }

    private final class TouchPadView extends TextView {
        private final TextPaint paint = new TextPaint(Paint.ANTI_ALIAS_FLAG);
        String caption = "";
        String status = "", hint = "";
        TouchPadView() { super(activity); setTextSize(14); }
        @Override protected void onDraw(Canvas canvas) {
            if (isSelected()) canvas.drawColor(accentSoft());
            paint.setColor(edge());
            int step = dp(16);
            for (int x = step / 2; x < getWidth(); x += step)
                for (int y = step / 2; y < getHeight(); y += step) canvas.drawCircle(x, y, dp(1), paint);
            paint.setColor(muted());
            paint.setTextSize(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, 11, getResources().getDisplayMetrics()));
            Drawable mouse = NativeIcon.of("mouse", muted(), dp(16));
            canvas.save();
            canvas.translate(dp(12), dp(10));
            mouse.draw(canvas);
            canvas.restore();
            int headerWidth = Math.max(0, getWidth() - dp(56));
            CharSequence name = TextUtils.ellipsize(caption, paint, headerWidth, TextUtils.TruncateAt.END);
            float baseline = dp(18) - (paint.ascent() + paint.descent()) / 2;
            canvas.drawText(name, 0, name.length(), dp(33), baseline, paint);
            float statusWidth = headerWidth - paint.measureText(name, 0, name.length());
            float stateWidth = paint.measureText(status);
            if (stateWidth <= statusWidth) {
                paint.setColor(isSelected() ? accent() : muted());
                canvas.drawText(status, getWidth() - dp(14) - stateWidth, baseline, paint);
            }
            paint.setColor(muted());
            drawGestureHint(canvas);
            int iconSize = dp(28);
            int iconBlock = iconSize + dp(7);
            Layout label = getLayout();
            int labelHeight = label == null ? 0 : label.getHeight();
            Drawable pointer = NativeIcon.of("pointer", isEnabled() ? accent() : muted(), iconSize);
            canvas.save();
            canvas.translate((getWidth() - iconSize) / 2f, (getHeight() - iconBlock - labelHeight) / 2f);
            pointer.draw(canvas);
            canvas.restore();
            canvas.save();
            canvas.translate(0, iconBlock / 2f);
            super.onDraw(canvas);
            canvas.restore();
        }

        private void drawGestureHint(Canvas canvas) {
            int width = getWidth() - dp(24);
            if (width <= 0 || hint.isEmpty()) return;
            float hintWidth = paint.measureText(hint);
            if (hintWidth <= width) {
                canvas.drawText(hint, (getWidth() - hintWidth) / 2, getHeight() - dp(12), paint);
                return;
            }
            StaticLayout layout = StaticLayout.Builder.obtain(hint, 0, hint.length(), paint, width)
                .setAlignment(Layout.Alignment.ALIGN_CENTER).setIncludePad(false)
                .setMaxLines(2).setEllipsize(TextUtils.TruncateAt.END).build();
            canvas.save();
            canvas.translate(dp(12), getHeight() - dp(9) - layout.getHeight());
            layout.draw(canvas);
            canvas.restore();
        }
    }

    private void forwardPointers(View view, MotionEvent event, String target) {
        int action = event.getActionMasked();
        if (action == MotionEvent.ACTION_MOVE) {
            if ("pad".equals(target) && event.getPointerCount() > 1) forwardTouchpadMove(event);
            else for (int i = 0; i < event.getPointerCount(); i++) pointer(view, event, target, "move", i);
        } else if (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) {
            pointer(view, event, target, "down", event.getActionIndex());
        } else if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_POINTER_UP) {
            if ("pad".equals(target) && Build.VERSION.SDK_INT >= 33 && (event.getFlags() & MotionEvent.FLAG_CANCELED) != 0) {
                pointer(view, event, target, "lost", event.getActionIndex());
                return;
            }
            pointer(view, event, target, "up", event.getActionIndex());
        } else if (action == MotionEvent.ACTION_CANCEL) pointer(view, event, target, "cancel", 0);
    }

    private JSONArray touchpadContacts(MotionEvent event) throws JSONException {
        float density = activity.getResources().getDisplayMetrics().density;
        JSONArray contacts = new JSONArray();
        for (int i = 0; i < event.getPointerCount(); i++) {
            contacts.put(new JSONObject().put("pointerId", event.getPointerId(i))
                .put("x", event.getX(i) / density).put("y", event.getY(i) / density));
        }
        return contacts;
    }

    private void forwardTouchpadMove(MotionEvent event) {
        try {
            host.pointer(new JSONObject().put("target", "pad").put("phase", "move-batch").put("contacts", touchpadContacts(event)));
        } catch (JSONException ignored) {}
    }

    private void pointer(View view, MotionEvent event, String target, String phase, int index) {
        float density = activity.getResources().getDisplayMetrics().density;
        try {
            JSONObject sample = new JSONObject().put("target", target).put("phase", phase)
                .put("pointerId", event.getPointerId(index)).put("primary", index == 0)
                .put("x", event.getX(index) / density)
                .put("y", ("mic".equals(target) ? event.getRawY() : event.getY(index)) / density)
                .put("width", view.getWidth() / density);
            host.pointer(sample);
        } catch (JSONException ignored) {}
    }

    private void renderSheet(JSONObject state) {
        String key = state.isNull("sheetKey") ? "" : state.optString("sheetKey");
        if (key.isEmpty()) { dismiss(); return; }
        boolean changed = !key.equals(sheetKey);
        boolean compactGrid = "more".equals(key) && wide && root.getHeight() <= dp(500);
        boolean densityChanged = compactMoreSheet != compactGrid;
        compactMoreSheet = compactGrid;
        sheetKey = key;
        if (dialog == null) createSheet();
        if (densityChanged) sheetContent.removeAllViews();
        if (changed || densityChanged) {
            sheetLayout.setPadding(dp(SHEET_GUTTER_DP), 0, dp(SHEET_GUTTER_DP), dp(compactGrid ? 8 : 24));
            sheetTitle.setPadding(0, 0, 0, dp(compactGrid ? 4 : 12));
            sheetScroll.scrollTo(0, 0);
            sheetLayout.expanded = false;
            setHandleLabel();
            sheetLayout.requestLayout();
        }
        if (sheetLayout.getBackground() == null || sheetLayout.darkSurface != dark) {
            sheetLayout.darkSurface = dark;
            GradientDrawable sheetBackground = shape(surface(), 28);
            int corner = dp(28);
            sheetBackground.setCornerRadii(new float[]{corner, corner, corner, corner, 0, 0, 0, 0});
            sheetLayout.setBackground(sheetBackground);
        }
        sheetTitle.setTextColor(ink());
        handle.invalidate();
        String title = state.optString("sheetTitle");
        if (Build.VERSION.SDK_INT >= 28 && !TextUtils.equals(title, sheetLayout.getAccessibilityPaneTitle()))
            sheetLayout.setAccessibilityPaneTitle(title);
        setText(sheetTitle, title);
        String preview = state.optString("sheetIcon");
        String previewStyle = preview + dark;
        if (!previewStyle.equals(sheetTitle.getTag())) {
            sheetTitle.setTag(previewStyle);
            sheetTitle.setCompoundDrawablePadding(dp(10));
            sheetTitle.setCompoundDrawablesRelative(NativeIcon.of(preview, accent(), dp(24)), null, null, null);
        }
        JSONArray items = state.optJSONArray("sheet");
        if ("more".equals(key) && items != null) items = moreLayout(items);
        if (key.startsWith("icon-") && items != null) items = iconLayout(items);
        reconcile(sheetContent, items, false);
    }

    private JSONArray iconLayout(JSONArray items) {
        int available = Math.min(root.getWidth() - root.getPaddingLeft() - root.getPaddingRight(),
            dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP)) - 2 * dp(SHEET_GUTTER_DP);
        int columns = AdaptiveLayout.fittingColumns(available, dp(MORE_GAP_DP), dp(74), 4);
        JSONArray result = new JSONArray(), row = new JSONArray();
        int line = 0;
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            if (!item.has("selected")) { result.put(item); continue; }
            if (row.length() == columns) row = new JSONArray();
            if (row.length() == 0) result.put(group("row", "more-icons-" + columns + "-" + line++, row));
            row.put(item);
        }
        return result;
    }

    private JSONArray moreLayout(JSONArray items) {
        int windowWidth = root.getWidth() - root.getPaddingLeft() - root.getPaddingRight();
        if (windowWidth <= 0) windowWidth = dp(activity.getResources().getConfiguration().screenWidthDp);
        int availableWidth = Math.min(windowWidth, dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP)) - 2 * dp(SHEET_GUTTER_DP);
        TextPaint paint = new TextPaint(Paint.ANTI_ALIAS_FLAG);
        paint.setTextSize(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, MORE_TILE_TEXT_SP,
            activity.getResources().getDisplayMetrics()));
        BreakIterator boundaries = BreakIterator.getLineInstance(Locale.ROOT);
        int minCellWidth = dp(AdaptiveLayout.TOUCH_TARGET_DP);
        JSONArray tiles = new JSONArray(), extras = new JSONArray();
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            if (item == null) continue;
            JSONArray row = item.optJSONArray("children");
            if (row == null || !item.optString("id").startsWith("more-")) {
                extras.put(item);
                continue;
            }
            for (int j = 0; j < row.length(); j++) {
                JSONObject tile = row.optJSONObject(j);
                if (tile == null) continue;
                tiles.put(tile);
                String label = tile.optString("label");
                boundaries.setText(label);
                int start = boundaries.first();
                for (int end = boundaries.next(); end != BreakIterator.DONE; start = end, end = boundaries.next()) {
                    int width = (int) Math.ceil(Layout.getDesiredWidth(label.substring(start, end).trim(), paint));
                    minCellWidth = Math.max(minCellWidth, width + 2 * dp(MORE_TILE_PADDING_DP));
                }
            }
        }
        int columns = AdaptiveLayout.fittingColumns(availableWidth, dp(MORE_GAP_DP), minCellWidth, 3);
        if (columns == 3) return items;
        JSONArray result = new JSONArray();
        String idPrefix = columns == 2 ? "more-double-" : "more-single-";
        for (int i = 0; i < tiles.length(); i += columns) {
            JSONArray row = new JSONArray();
            for (int j = i; j < Math.min(i + columns, tiles.length()); j++) row.put(tiles.optJSONObject(j));
            result.put(group("row", idPrefix + i, row));
        }
        for (int i = 0; i < extras.length(); i++) result.put(extras.optJSONObject(i));
        return result;
    }

    private void createSheet() {
        dialog = new Dialog(activity);
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE);
        sheetLayout = new SheetLayout();
        sheetLayout.setOrientation(LinearLayout.VERTICAL);
        sheetLayout.setFocusableInTouchMode(true);
        sheetLayout.setPadding(dp(SHEET_GUTTER_DP), 0, dp(SHEET_GUTTER_DP), dp(24));
        sheetLayout.setOnApplyWindowInsetsListener((view, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                boolean visible = insets.isVisible(android.view.WindowInsets.Type.ime());
                if (sheetLayout.imeVisible != visible) {
                    sheetLayout.imeVisible = visible;
                    sheetLayout.requestLayout();
                    if (visible && "keyboard".equals(sheetKey))
                        sheetScroll.post(() -> sheetScroll.smoothScrollTo(0, sheetContent.getHeight()));
                }
            }
            return view.onApplyWindowInsets(insets);
        });
        handle = new View(activity) {
            private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            @Override public CharSequence getAccessibilityClassName() { return Button.class.getName(); }
            @Override protected void onDraw(Canvas canvas) {
                paint.setColor(muted());
                paint.setAlpha(isPressed() || isFocused() ? 220 : 160);
                float x = getWidth() / 2f, y = getHeight() / 2f;
                canvas.drawRoundRect(x - dp(16), y - dp(2), x + dp(16), y + dp(2), dp(2), dp(2), paint);
            }
            @Override protected void drawableStateChanged() {
                super.drawableStateChanged();
                invalidate();
            }
        };
        handle.setFocusable(true);
        handle.setOnClickListener(v -> { if (sheetLayout.expandable) expandSheet(); else dialog.cancel(); });
        setHandleLabel();
        handle.setOnTouchListener(new View.OnTouchListener() {
            private float start;
            private int startHeight;
            @Override public boolean onTouch(View view, MotionEvent event) {
                switch (event.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        sheetLayout.animate().cancel();
                        start = event.getRawY();
                        startHeight = sheetLayout.getHeight();
                        view.setPressed(true);
                        return true;
                    case MotionEvent.ACTION_MOVE:
                        float delta = event.getRawY() - start;
                        if (sheetLayout.expandable) {
                            sheetLayout.dragHeight = Math.max(sheetLayout.collapsedHeight,
                                Math.min(sheetLayout.fullHeight, Math.round(startHeight - delta)));
                            sheetLayout.requestLayout();
                        }
                        float consumed = sheetLayout.expandable ? startHeight - sheetLayout.dragHeight : 0;
                        sheetLayout.setTranslationY(Math.max(0, delta - consumed) * .55f);
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        view.setPressed(false);
                        resetSheetOffset();
                        return true;
                    case MotionEvent.ACTION_UP:
                        float distance = event.getRawY() - start;
                        view.setPressed(false);
                        resetSheetOffset();
                        if (distance > dp(56)) {
                            if (sheetLayout.expanded) expandSheet();
                            else dialog.cancel();
                        } else if (distance < -dp(56)) {
                            if (sheetLayout.expandable && !sheetLayout.expanded) expandSheet();
                        } else if (Math.abs(distance) < dp(12)) view.performClick();
                        return true;
                    default: return true;
                }
            }
        });
        sheetLayout.addView(handle, new LinearLayout.LayoutParams(-1, dp(48)));
        sheetTitle = label(20);
        sheetTitle.setPadding(0, 0, 0, dp(12));
        sheetTitle.setTypeface(null, Typeface.BOLD);
        markHeading(sheetTitle);
        sheetLayout.addView(sheetTitle);
        sheetScroll = new ScrollView(activity);
        sheetContent = column();
        sheetScroll.addView(sheetContent);
        sheetLayout.addView(sheetScroll, new LinearLayout.LayoutParams(-1, -2));
        dialog.setContentView(sheetLayout);
        dialog.setCanceledOnTouchOutside(true);
        dialog.setOnCancelListener(ignored -> emit("dismiss", null));
        Window window = dialog.getWindow();
        window.setGravity(Gravity.BOTTOM);
        window.setBackgroundDrawableResource(android.R.color.transparent);
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE | WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
        window.addFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND);
        window.setDimAmount(.40f);
        dialog.show();
        resizeDialog();
    }

    private void setHandleLabel() {
        handle.setContentDescription(AppLanguage.text(activity, !sheetLayout.expandable ? R.string.ui_close_panel : sheetLayout.expanded ? R.string.ui_collapse_sheet : R.string.ui_expand_sheet));
    }
    private void resetSheetOffset() {
        sheetLayout.dragHeight = -1;
        sheetLayout.requestLayout();
        if (android.animation.ValueAnimator.areAnimatorsEnabled())
            sheetLayout.animate().translationY(0).setDuration(140).start();
        else sheetLayout.setTranslationY(0);
    }
    private void expandSheet() {
        sheetLayout.expanded = !sheetLayout.expanded;
        setHandleLabel();
        sheetLayout.requestLayout();
    }
    private void resizeDialog() {
        int maxWidth = dp(AdaptiveLayout.SHEET_MAX_WIDTH_DP);
        if (dialog == null) return;
        Window window = dialog.getWindow();
        int width = root.getWidth() > maxWidth ? maxWidth : ViewGroup.LayoutParams.MATCH_PARENT;
        WindowManager.LayoutParams params = window.getAttributes();
        if (params.width != width || params.height != ViewGroup.LayoutParams.WRAP_CONTENT) {
            window.setLayout(width, ViewGroup.LayoutParams.WRAP_CONTENT);
        }
    }
    private void dismiss() {
        if (dialog != null) { dialog.dismiss(); dialog = null; }
        sheetKey = "";
    }
    void destroy() {
        if (permissionHint != null) permissionHint.dismiss();
        dismiss();
        if (microphoneCapsule != null) root.removeView(microphoneCapsule);
        root.removeView(screen);
    }
    private final class KeyStrip extends HorizontalScrollView {
        private final Runnable alignKeys = this::alignToKey;
        private int touchStart;
        private boolean waitingToAlign;

        KeyStrip() { super(activity); }

        @Override public boolean dispatchTouchEvent(MotionEvent event) {
            int action = event.getActionMasked();
            if (action == MotionEvent.ACTION_DOWN) {
                touchStart = getScrollX();
                waitingToAlign = false;
                removeCallbacks(alignKeys);
            }
            boolean handled = super.dispatchTouchEvent(event);
            if ((action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) && touchStart != getScrollX()) {
                waitingToAlign = true;
                postDelayed(alignKeys, 100);
            }
            return handled;
        }

        @Override protected void onScrollChanged(int x, int y, int oldX, int oldY) {
            super.onScrollChanged(x, y, oldX, oldY);
            if (waitingToAlign) {
                removeCallbacks(alignKeys);
                postDelayed(alignKeys, 100);
            }
        }

        private void alignToKey() {
            waitingToAlign = false;
            if (!(getChildAt(0) instanceof LinearLayout buttons)) return;
            int maxScroll = Math.max(0, buttons.getWidth() - getWidth());
            int target = maxScroll;
            for (int i = 0; i < buttons.getChildCount(); i++) {
                int start = Math.min(maxScroll, buttons.getChildAt(i).getLeft());
                if (Math.abs(start - getScrollX()) < Math.abs(target - getScrollX())) target = start;
            }
            if (target != getScrollX()) smoothScrollTo(target, 0);
        }

        @Override protected void onDetachedFromWindow() {
            waitingToAlign = false;
            removeCallbacks(alignKeys);
            super.onDetachedFromWindow();
        }
    }

    private final class KeyboardGrid extends LinearLayout {
        private int columns, cellHeight;
        KeyboardGrid() { super(activity); }

        @Override protected void onMeasure(int widthSpec, int heightSpec) {
            int width = MeasureSpec.getSize(widthSpec), gap = dp(6), count = getChildCount();
            int minWidth = dp(48), natural = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED);
            cellHeight = dp(48);
            for (int i = 0; i < count; i++) {
                View key = getChildAt(i);
                key.measure(natural, natural);
                minWidth = Math.max(minWidth, key.getMeasuredWidth());
                cellHeight = Math.max(cellHeight, key.getMeasuredHeight());
            }
            int fit = Math.max(1, (width + gap) / (minWidth + gap));
            int rows = Math.max(1, (count + fit - 1) / fit);
            columns = Math.max(1, (count + rows - 1) / rows);
            int cellWidth = Math.max(0, (width - (columns - 1) * gap) / columns);
            for (int i = 0; i < count; i++)
                getChildAt(i).measure(MeasureSpec.makeMeasureSpec(cellWidth, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(cellHeight, MeasureSpec.EXACTLY));
            setMeasuredDimension(width, rows * cellHeight + (rows - 1) * gap);
        }

        @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
            int gap = dp(6);
            for (int i = 0; i < getChildCount(); i++) {
                View key = getChildAt(i);
                int x = (i % columns) * (key.getMeasuredWidth() + gap);
                int y = (i / columns) * (cellHeight + gap);
                key.layout(x, y, x + key.getMeasuredWidth(), y + cellHeight);
            }
        }
    }

    private final class SessionEdit extends EditText {
        int revision;
        SessionEdit() { super(activity); }

        @Override protected void onMeasure(int width, int height) {
            if (getParent() instanceof View parent && "column:keyboard-entry".equals(parent.getTag())) {
                int space = Math.max(dp(64), Math.min(dp(224), viewportHeight / 3));
                if (shortKeyboard) space = Math.max(dp(48), Math.min(dp(96), viewportHeight - dp(14)));
                height = MeasureSpec.makeMeasureSpec(space, MeasureSpec.EXACTLY);
            }
            super.onMeasure(width, height);
        }
    }
    private final class SheetLayout extends LinearLayout {
        private boolean expanded, expandable;
        private int collapsedHeight, fullHeight, dragHeight = -1;
        private boolean imeVisible;
        private boolean darkSurface;
        SheetLayout() { super(activity); }
        @Override public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info) {
            super.onInitializeAccessibilityNodeInfo(info);
            info.setDismissable(true);
            info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_DISMISS);
        }
        @Override public boolean performAccessibilityAction(int action, android.os.Bundle arguments) {
            if (action == AccessibilityNodeInfo.AccessibilityAction.ACTION_DISMISS.getId() && dialog != null) {
                dialog.cancel();
                return true;
            }
            return super.performAccessibilityAction(action, arguments);
        }
        @Override protected void onMeasure(int width, int height) {
            int available = MeasureSpec.getSize(height);
            int visible = Math.min(available, root.getHeight() > 0 ? root.getHeight() : available);
            int limit = Math.max(0, visible - dp(12));
            View draft = sheetContent == null ? null : sheetContent.findViewWithTag("edit:draft");
            android.graphics.Rect viewport = new android.graphics.Rect();
            getWindowVisibleDisplayFrame(viewport);
            boolean typing = imeVisible || (viewport.height() > 0 && root.getHeight() - viewport.height() >= dp(120));
            int fieldHeight = typing ? dp(64) : Math.max(dp(64), Math.min(dp(160), limit - dp(330)));
            if (draft instanceof EditText && draft.getMinimumHeight() != fieldHeight) draft.setMinimumHeight(fieldHeight);
            super.onMeasure(width, MeasureSpec.makeMeasureSpec(limit, MeasureSpec.AT_MOST));
            fullHeight = getMeasuredHeight();
            collapsedHeight = Math.min(fullHeight, typing || compactMoreSheet ? limit : Math.round(visible * .88f));
            boolean canExpand = fullHeight - collapsedHeight > dp(2);
            if (expandable != canExpand) {
                expandable = canExpand;
                if (!expandable) expanded = false;
                setHandleLabel();
            }
            int target = dragHeight >= 0 ? Math.min(fullHeight, Math.max(collapsedHeight, dragHeight))
                : expanded ? fullHeight : collapsedHeight;
            if (target != fullHeight) super.onMeasure(width, MeasureSpec.makeMeasureSpec(target, MeasureSpec.EXACTLY));
        }
    }
}
