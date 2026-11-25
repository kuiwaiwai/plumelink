package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.accessibility.AccessibilityNodeInfo;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import org.json.JSONObject;

/** Native presentations of the shared microphone gesture state. */
final class NativeMicrophoneUi {
    private final Context context;
    NativeMicrophoneUi(Context context) { this.context = context; }
    private int dp(float n) { return Math.round(n * context.getResources().getDisplayMetrics().density); }
    private int accent(boolean dark) { return UiStyle.accent(dark); }
    private int soft(boolean dark) { return UiStyle.accentSoft(dark); }
    private int foreground(boolean dark) { return UiStyle.ink(dark); }
    private int background(boolean dark) { return UiStyle.background(dark); }
    private int onAccent(boolean dark) { return UiStyle.onAccent(dark); }
    private int danger(boolean dark) { return UiStyle.danger(dark); }
    private GradientDrawable shape(int fill, int radius) {
        GradientDrawable draw = new GradientDrawable();
        draw.setCornerRadius(dp(radius));
        draw.setColor(fill);
        return draw;
    }
    private TextView label(int size, boolean strong) {
        TextView view = new TextView(context);
        view.setTextSize(size);
        if (strong) view.setTypeface(null, Typeface.BOLD);
        return view;
    }
    private void setText(TextView view, String value) {
        if (!value.contentEquals(view.getText())) view.setText(value);
    }
    private void icon(Canvas canvas, NativeIcon.Cache cache, String name, int color, float x, float y, int size) {
        Drawable glyph = cache.get(name, color, dp(size));
        if (glyph == null) return;
        canvas.save();
        canvas.translate(x - dp(size) / 2f, y - dp(size) / 2f);
        glyph.draw(canvas);
        canvas.restore();
    }
    private void ring(Canvas canvas, Paint brush, float x, float y, int radius, int color, int progress) {
        if (progress <= 0) return;
        float r = dp(radius);
        brush.setStyle(Paint.Style.STROKE);
        brush.setStrokeWidth(dp(2.5f));
        brush.setStrokeCap(Paint.Cap.ROUND);
        brush.setColor(color);
        canvas.drawArc(x - r, y - r, x + r, y + r, -90, 3.6f * progress, false, brush);
        brush.setStyle(Paint.Style.FILL);
    }

    final class Key extends TextView {
        private final Paint brush = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final NativeIcon.Cache glyph = new NativeIcon.Cache(), lockGlyph = new NativeIcon.Cache();
        private boolean locked, stopping, dark, recording, tablet, iconOnly;
        private int progress;
        private int idleIconColor;
        private int lastFill = Integer.MIN_VALUE;

        Key() { super(context); }
        @Override public CharSequence getAccessibilityClassName() { return Button.class.getName(); }
        void update(JSONObject state, boolean night, boolean wideTablet, int idleIconColor, boolean iconOnly) {
            this.iconOnly = iconOnly;
            dark = night;
            this.idleIconColor = idleIconColor;
            tablet = wideTablet;
            locked = state.optBoolean("locked");
            stopping = "stop".equals(state.optString("intent"));
            recording = state.optBoolean("recording");
            progress = state.optInt("progress");
            int fill = stopping ? danger(dark) : locked ? accent(dark) : recording ? soft(dark)
                : iconOnly ? (dark ? 0xff252a35 : 0xffeceff5) : tablet ? (dark ? 0xff1b1e26 : 0xffffffff) : 0;
            if (lastFill != fill) { lastFill = fill; setBackground(shape(fill, iconOnly ? 20 : 16)); }
            setTextColor(stopping || locked ? onAccent(dark) : recording ? accent(dark) : foreground(dark));
            invalidate();
        }
        @Override protected void onDraw(Canvas canvas) {
            canvas.save();
            if (!iconOnly) canvas.translate(0, NativeRemoteUi.centeredStackOffset(this));
            float x = getWidth() / 2f;
            float y = iconOnly ? getHeight() / 2f : getPaddingTop() - dp(5) - dp(22) / 2f;
            int color = stopping || locked || recording ? getCurrentTextColor() : idleIconColor;
            icon(canvas, glyph, stopping ? "mic-off" : "mic", color, x, y, 22);
            if (locked && !stopping) icon(canvas, lockGlyph, "lock", color, x + dp(14), y - dp(10), 11);
            ring(canvas, brush, x, y, 19, color, progress);
            super.onDraw(canvas);
            canvas.restore();
        }
    }

    final class Capsule extends LinearLayout {
        private final Signal lockTarget = new Signal(0);
        private final TextView heading = label(13, true), detail = label(10, false);
        private final Signal meter = new Signal(1);
        private int lastFill = Integer.MIN_VALUE;
        Capsule() {
            super(context);
            setGravity(Gravity.CENTER_VERTICAL);
            setPadding(dp(16), dp(16), dp(16), dp(16));
            setElevation(dp(8));
            setClickable(false);
            addView(lockTarget, new LayoutParams(dp(44), dp(44)));
            LinearLayout copy = new LinearLayout(context);
            copy.setOrientation(VERTICAL);
            copy.addView(heading);
            detail.setPadding(0, dp(4), 0, 0);
            copy.addView(detail);
            LayoutParams copyParams = new LayoutParams(0, -2, 1);
            copyParams.leftMargin = copyParams.rightMargin = dp(12);
            addView(copy, copyParams);
            addView(meter, new LayoutParams(dp(39), dp(30)));
        }
        void update(JSONObject state, boolean dark) {
            int fill = foreground(dark), ink = background(dark);
            if (lastFill != fill) { lastFill = fill; setBackground(shape(fill, 22)); }
            heading.setTextColor(ink);
            detail.setTextColor(ink);
            detail.setAlpha(.72f);
            setText(heading, state.optString("label"));
            setText(detail, state.optString("detail"));
            lockTarget.update(state, dark, ink);
            meter.update(state, dark, ink);
            meter.setContentDescription(state.optString("meterLabel") + " " + state.optInt("level") + "%");
        }
    }

    final class HeaderStatus extends LinearLayout {
        private final Signal mic = new Signal(2), meter = new Signal(3), stop = new Signal(4);
        private final TextView elapsed = label(12, true);

        HeaderStatus(Runnable onStop) {
            super(context);
            setGravity(Gravity.CENTER_VERTICAL);
            addView(mic, new LayoutParams(dp(14), dp(16)));
            elapsed.setSingleLine();
            elapsed.setFontFeatureSettings("tnum");
            elapsed.setEllipsize(android.text.TextUtils.TruncateAt.END);
            elapsed.setPadding(dp(3), 0, dp(3), 0);
            addView(elapsed, new LayoutParams(-2, -2));
            addView(meter, new LayoutParams(dp(12), dp(16)));
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_YES);
            stop.setFocusable(true);
            stop.setOnClickListener(v -> onStop.run());
        }

        View stopButton() { return stop; }

        @Override protected void onMeasure(int widthMeasureSpec, int heightMeasureSpec) {
            int available = MeasureSpec.getMode(widthMeasureSpec) == MeasureSpec.UNSPECIFIED ? Integer.MAX_VALUE
                : Math.max(0, MeasureSpec.getSize(widthMeasureSpec) - getPaddingLeft() - getPaddingRight()
                    - mic.getLayoutParams().width - meter.getLayoutParams().width);
            if (elapsed.getMaxWidth() != available) elapsed.setMaxWidth(available);
            super.onMeasure(widthMeasureSpec, heightMeasureSpec);
        }

        void update(JSONObject state, boolean dark) {
            int ink = accent(dark);
            String status = state.optString("compactLabel"), duration = state.optString("duration");
            setText(elapsed, state.optBoolean("warning") || !state.optBoolean("recording") ? status : duration);
            elapsed.setTextColor(ink);
            String description = status + " " + duration;
            if (!android.text.TextUtils.equals(getContentDescription(), description)) setContentDescription(description);
            mic.update(state, dark, ink);
            meter.update(state, dark, ink);
            meter.setContentDescription(state.optString("meterLabel"));
            stop.update(state, dark, ink);
            stop.setContentDescription(state.optString("stopLabel"));
        }
    }

    private final class Signal extends View {
        private static final float[] BARS = {.45f, .75f, 1f, .65f, .9f, .55f, .35f};
        private static final float[] COMPACT_BARS = {.5f, 1f, .7f};
        private final Paint brush = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final NativeIcon.Cache glyph = new NativeIcon.Cache();
        private final int kind;
        private int ink, level, progress;
        private boolean locking, stopping, warning, dark;
        Signal(int kind) { super(context); this.kind = kind; }
        @Override public CharSequence getAccessibilityClassName() {
            return kind == 4 ? Button.class.getName() : super.getAccessibilityClassName();
        }
        @Override public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info) {
            super.onInitializeAccessibilityNodeInfo(info);
            if (kind == 3) info.setRangeInfo(AccessibilityNodeInfo.RangeInfo.obtain(
                AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_INT, 0, 100, level));
        }
        void update(JSONObject state, boolean night, int color) {
            ink = color;
            dark = night;
            level = state.optInt("level");
            progress = state.optInt("progress");
            locking = "lock".equals(state.optString("intent"));
            stopping = "stop".equals(state.optString("intent"));
            warning = state.optBoolean("warning");
            invalidate();
        }
        @Override protected void onDraw(Canvas canvas) {
            float x = getWidth() / 2f, y = getHeight() / 2f;
            if (kind == 0) {
                brush.setColor(locking ? accent(dark) : dark ? 0xffd2d6df : 0xff333945);
                canvas.drawCircle(x, y, dp(22), brush);
                icon(canvas, glyph, locking ? "lock" : "up", locking ? onAccent(dark) : ink, x, y, 20);
                if (locking) ring(canvas, brush, x, y, 20, onAccent(dark), progress);
            } else if (kind == 2) {
                icon(canvas, glyph, warning ? "mic-off" : "mic", ink, x, y, 16);
            } else if (kind == 4) {
                brush.setColor(soft(dark));
                canvas.drawCircle(x, y, dp(15), brush);
                brush.setColor(ink);
                canvas.drawRoundRect(x - dp(5), y - dp(5), x + dp(5), y + dp(5), dp(2), dp(2), brush);
                if (stopping) ring(canvas, brush, x, y, 16, ink, progress);
            } else {
                float[] weights = kind == 3 ? COMPACT_BARS : BARS;
                float pitch = getWidth() / (float) weights.length, width = dp(kind == 3 ? 2 : 3);
                brush.setColor(ink);
                for (int i = 0; i < weights.length; i++) {
                    float radius = y * (.12f + level / 100f * weights[i] * .88f);
                    float barX = pitch * (i + .5f);
                    canvas.drawRoundRect(barX - width / 2, y - radius, barX + width / 2, y + radius, width, width, brush);
                }
            }
        }
    }
}
