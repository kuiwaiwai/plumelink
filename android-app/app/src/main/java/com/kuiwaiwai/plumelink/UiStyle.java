package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.graphics.drawable.StateListDrawable;
import android.view.Gravity;
import android.widget.EditText;
import android.widget.TextView;

final class UiStyle {
    static final int BUTTON_RADIUS = 16;
    static final int FIELD_RADIUS = 14;
    static final int SHEET_RADIUS = 28;
    static final int SHEET_GUTTER = 20;
    static final int ICON = 20;
    private static final java.util.WeakHashMap<EditText, Boolean> fieldThemes = new java.util.WeakHashMap<>();

    private UiStyle() {}
    static int dp(Context context, float value) { return Math.round(value * context.getResources().getDisplayMetrics().density); }
    static int background(boolean dark) { return dark ? 0xff111318 : 0xfff5f6fa; }
    static int surface(boolean dark) { return dark ? 0xff1b1e26 : Color.WHITE; }
    static int tonal(boolean dark) { return dark ? 0xff252a35 : 0xffeceff5; }
    static int ink(boolean dark) { return dark ? 0xffedf0f7 : 0xff1a202c; }
    static int muted(boolean dark) { return dark ? 0xffa3adbf : 0xff5f6b7e; }
    static int edge(boolean dark) { return dark ? 0xff303642 : 0xffe6e9f0; }
    static int accent(boolean dark) { return dark ? 0xffa5bdff : 0xff2868e8; }
    static int accentSoft(boolean dark) { return dark ? 0xff283b62 : 0xffe6edff; }
    static int onAccent(boolean dark) { return dark ? 0xff122448 : Color.WHITE; }
    static int danger(boolean dark) { return dark ? 0xffffb1b6 : 0xffb73b45; }
    static int dangerSoft(boolean dark) { return dark ? 0xff362127 : 0xfffff0f1; }

    static GradientDrawable shape(Context context, int fill, int radius) {
        GradientDrawable shape = new GradientDrawable();
        shape.setColor(fill);
        shape.setCornerRadius(dp(context, radius));
        return shape;
    }

    static RippleDrawable ripple(Context context, boolean dark, int fill, int radius) {
        ColorStateList states = new ColorStateList(new int[][]{
            {android.R.attr.state_pressed}, {android.R.attr.state_focused}, {android.R.attr.state_hovered}, {}
        }, new int[]{dark ? 0x30ffffff : 0x240051dc, dark ? 0x28a5bdff : 0x202868e8,
            dark ? 0x1affffff : 0x142868e8, dark ? 0x30ffffff : 0x240051dc});
        return new RippleDrawable(states, shape(context, fill, radius), shape(context, Color.WHITE, radius));
    }

    static void button(TextView button) {
        Context context = button.getContext();
        button.setAllCaps(false);
        button.setTextSize(14);
        button.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        button.setLetterSpacing(0);
        button.setIncludeFontPadding(false);
        button.setMinWidth(0); button.setMinimumWidth(0);
        button.setMinHeight(dp(context, 48)); button.setMinimumHeight(dp(context, 48));
        button.setGravity(Gravity.CENTER);
        button.setPadding(dp(context, 16), dp(context, 12), dp(context, 16), dp(context, 12));
        button.setStateListAnimator(null);
        button.setElevation(0);
        button.setBackgroundTintList(null);
    }

    static void field(EditText field, boolean dark) {
        Context context = field.getContext();
        if (!Boolean.valueOf(dark).equals(fieldThemes.get(field))) {
            StateListDrawable states = new StateListDrawable();
            GradientDrawable focus = shape(context, tonal(dark), FIELD_RADIUS);
            focus.setStroke(dp(context, 1), accent(dark));
            GradientDrawable idle = shape(context, tonal(dark), FIELD_RADIUS);
            idle.setStroke(dp(context, 1), edge(dark));
            states.addState(new int[]{android.R.attr.state_focused}, focus);
            states.addState(new int[]{}, idle);
            field.setBackgroundTintList(null);
            field.setBackground(states);
            fieldThemes.put(field, dark);
        }
        field.setTextSize(16);
        field.setLetterSpacing(0);
        field.setTextColor(ink(dark));
        field.setHintTextColor(muted(dark));
        field.setMinimumHeight(dp(context, 52));
        field.setPadding(dp(context, 14), dp(context, 12), dp(context, 14), dp(context, 12));
    }
}
