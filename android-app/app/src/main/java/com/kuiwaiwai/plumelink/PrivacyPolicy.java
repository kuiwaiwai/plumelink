package com.kuiwaiwai.plumelink;

import android.app.Activity;
import android.graphics.Typeface;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.style.StyleSpan;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;

final class PrivacyPolicy {
    private PrivacyPolicy() {}

    static CharSequence content(Activity activity) {
        String section = "## " + AppLanguage.text(activity, R.string.ui_privacy_language);
        SpannableStringBuilder body = new SpannableStringBuilder();
        try (BufferedReader reader = new BufferedReader(new InputStreamReader(
                activity.getAssets().open("PRIVACY.md"), StandardCharsets.UTF_8))) {
            boolean selected = false;
            String line;
            while ((line = reader.readLine()) != null) {
                if (line.startsWith("## ")) {
                    if (selected) break;
                    selected = line.equals(section);
                } else if (selected && !line.isEmpty()) {
                    if (body.length() > 0) body.append("\n\n");
                    if (line.startsWith("### ")) {
                        body.append(line.substring(4), new StyleSpan(Typeface.BOLD), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
                    } else {
                        body.append(line);
                    }
                }
            }
        } catch (IOException e) {
            android.util.Log.e("PlumeLink", "Could not read packaged privacy policy", e);
        }
        return body.length() == 0 ? AppLanguage.text(activity, R.string.ui_privacy_unavailable) : body;
    }
}
