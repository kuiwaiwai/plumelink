package com.kuiwaiwai.plumelink;

import android.content.Context;
import java.io.IOException;
import java.util.Locale;

/** A transfer failure with a user-facing reason in the current app language. */
class TransferFailure extends IOException {
    private final int reason;
    private final Object[] arguments;

    TransferFailure(int reason, Object... arguments) {
        this.reason = reason;
        this.arguments = arguments;
    }

    String message(Context context) {
        String template = AppLanguage.text(context, reason);
        return arguments.length == 0 ? template : String.format(Locale.ROOT, template, arguments);
    }
}
