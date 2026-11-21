package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.content.res.Configuration;
import java.util.Locale;

final class AppLanguage {
    static final String[] OPTIONS = {"system", "zh-CN", "zh-TW", "en"};
    private AppLanguage() {}

    static String preference(Context context) {
        return context.getSharedPreferences("appearance", Context.MODE_PRIVATE).getString("language", "system");
    }

    static void set(Context context, String value) {
        for (String option : OPTIONS) {
            if (option.equals(value)) {
                context.getSharedPreferences("appearance", Context.MODE_PRIVATE).edit().putString("language", value).apply();
                return;
            }
        }
    }

    static String text(Context context, int id) {
        String value = preference(context);
        if ("system".equals(value)) return context.getString(id);
        Configuration configuration = new Configuration(context.getResources().getConfiguration());
        configuration.setLocale(Locale.forLanguageTag(value));
        return context.createConfigurationContext(configuration).getString(id);
    }
}
