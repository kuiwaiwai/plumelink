package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.content.SharedPreferences;
import android.content.res.Configuration;
import android.app.AlertDialog;

final class AppAppearance {
    private static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences("appearance", Context.MODE_PRIVATE);
    }
    static String preference(Context context) { return prefs(context).getString("theme", ""); }
    static void save(Context context, String value) {
        if (("system".equals(value) || "dark".equals(value) || "light".equals(value)) && !value.equals(preference(context)))
            prefs(context).edit().putString("theme", value).apply();
    }
    static boolean dark(Context context) {
        String value = preference(context);
        if ("dark".equals(value)) return true;
        if ("light".equals(value)) return false;
        return systemDark(context);
    }
    static boolean systemDark(Context context) {
        return (context.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }
    static AlertDialog.Builder dialog(Context context) {
        return new AlertDialog.Builder(context, dark(context) ? android.R.style.Theme_Material_Dialog_Alert : android.R.style.Theme_Material_Light_Dialog_Alert);
    }
}
