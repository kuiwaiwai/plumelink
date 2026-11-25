package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.widget.FrameLayout;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;
import java.util.function.Consumer;

final class AppWindowLayout extends FrameLayout {
    private boolean keyboardVisible;
    private Consumer<Boolean> keyboardListener;

    AppWindowLayout(Context context) {
        super(context);
        ViewCompat.setOnApplyWindowInsetsListener(this, (view, insets) -> {
            Insets bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
            Insets keyboard = insets.getInsets(WindowInsetsCompat.Type.ime());
            boolean visible = insets.isVisible(WindowInsetsCompat.Type.ime());
            if (keyboardVisible != visible) {
                keyboardVisible = visible;
                if (keyboardListener != null) keyboardListener.accept(visible);
            }
            view.setPadding(bars.left, bars.top, bars.right, Math.max(bars.bottom, keyboard.bottom));
            return WindowInsetsCompat.CONSUMED;
        });
    }

    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        ViewCompat.requestApplyInsets(this);
    }

    boolean keyboardVisible() { return keyboardVisible; }

    void setKeyboardListener(Consumer<Boolean> listener) {
        keyboardListener = listener;
        if (listener != null) listener.accept(keyboardVisible);
    }
}
