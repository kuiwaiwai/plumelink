package com.kuiwaiwai.plumelink;

import android.graphics.Canvas;
import android.graphics.ColorFilter;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.graphics.drawable.Drawable;
import androidx.core.graphics.PathParser;
import java.util.HashMap;

/** Lucide outlines shared with the web interface (ISC license, see THIRD_PARTY_NOTICES). */
final class NativeIcon extends Drawable {
    private static final HashMap<String, Path> PATHS = new HashMap<>();
    private final Path path;
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);

    private NativeIcon(Path path, int color, int size) {
        this.path = path;
        paint.setColor(color);
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(1.8f);
        paint.setStrokeCap(Paint.Cap.ROUND);
        paint.setStrokeJoin(Paint.Join.ROUND);
        setBounds(0, 0, size, size);
    }

    static Drawable of(String id, int color, int size) {
        if (id == null) return null;
        String data = data(id);
        if (data == null) return null;
        Path path = PATHS.computeIfAbsent(id, key -> {
            try {
                return parse(key);
            } catch (RuntimeException error) {
                android.util.Log.w("PlumeLink", "Could not load icon " + key, error);
                return PathParser.createPathFromPathData("M4 4H20V20H4Z M8 12H16");
            }
        });
        return new NativeIcon(path, color, size);
    }

    static Path parse(String id) { return PathParser.createPathFromPathData(data(id)); }

    @Override public void draw(Canvas canvas) {
        canvas.save();
        canvas.translate(getBounds().left, getBounds().top);
        canvas.scale(getBounds().width() / 24f, getBounds().height() / 24f);
        canvas.drawPath(path, paint);
        canvas.restore();
    }

    @Override public void setAlpha(int value) { paint.setAlpha(value); }
    @Override public void setColorFilter(ColorFilter filter) { paint.setColorFilter(filter); }
    @Override public int getOpacity() { return PixelFormat.TRANSLUCENT; }

    private static String data(String id) {
        switch (id) {
            case "mic-off": return "M2 2L22 22 M9 9V11a3 3 0 0 0 5 2 M15 9V5a3 3 0 0 0 -5.9 -.8 M5 10v2a7 7 0 0 0 12 4.9 M19 10v2 M12 19v3";
            case "lock": return "M7 11V7a5 5 0 0 1 10 0v4 M5 11h14a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1Z M12 15v2";
            case "close": return "M6 6L18 18 M18 6L6 18";
            case "escape": return "M4 10V4h6 M4 4l9 9 M13 5a8 8 0 1 1 -8 8";
            case "back": return "M9 14 4 9l5-5 M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1 -5.5 5.5H11";
            case "taskview": return "M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2H10a2 2 0 0 1 -2 -2V10a2 2 0 0 1 2 -2z M4 16a2 2 0 0 1 -2 -2V4a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2";
            case "pointer": return "M4.037 4.688a.495 .495 0 0 1 .651 -.651l16 6.5a.5 .5 0 0 1 -.063 .947l-6.124 1.58a2 2 0 0 0 -1.438 1.435l-1.579 6.126a.5 .5 0 0 1 -.947 .063z";
            case "files": return "M15 2h-4a2 2 0 0 0 -2 2v11a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2V8 M16.706 2.706A2.4 2.4 0 0 0 15 2v5a1 1 0 0 0 1 1h5a2.4 2.4 0 0 0 -.706 -1.706z M5 7a2 2 0 0 0 -2 2v11a2 2 0 0 0 2 2h8a2 2 0 0 0 1.732 -1";
            case "settings": return "M14 17H5 M19 7h-9 M14,17 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0 Z M4,7 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0 Z";
            case "mouse": return "M12,2 H12 Q19,2 19,9 V15 Q19,22 12,22 H12 Q5,22 5,15 V9 Q5,2 12,2 Z M12 6v4";
            case "enter": return "M20 4v7a4 4 0 0 1 -4 4H4 M0 0 m9 10-5 5 5 5";
            case "keyboard": return "M10 8h.01 M12 12h.01 M14 8h.01 M16 12h.01 M18 8h.01 M6 8h.01 M7 16h10 M8 12h.01 M4,4 H20 Q22,4 22,6 V18 Q22,20 20,20 H4 Q2,20 2,18 V6 Q2,4 4,4 Z";
            case "mic": return "M12 19v3 M19 10v2a7 7 0 0 1 -14 0v-2 M12,2 H12 Q15,2 15,5 V12 Q15,15 12,15 H12 Q9,15 9,12 V5 Q9,2 12,2 Z";
            case "more": return "M11,12 a1 1 0 1 0 2 0 a1 1 0 1 0 -2 0 Z M18,12 a1 1 0 1 0 2 0 a1 1 0 1 0 -2 0 Z M4,12 a1 1 0 1 0 2 0 a1 1 0 1 0 -2 0 Z";
            case "quieter": return "M11 4.702a.705 .705 0 0 0 -1.203 -.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0 -1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997 .413l3.383 3.384A.705 .705 0 0 0 11 19.298z M16 9a5 5 0 0 1 0 6";
            case "mute": return "M11 4.702a.7 .7 0 0 0 -1.203 -.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0 -1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997 .413l3.383 3.384A.7 .7 0 0 0 11 19.298z M0 0 m16.5 14.5 5-5 M0 0 m16.5 9.5 5 5";
            case "louder": return "M11 4.702a.705 .705 0 0 0 -1.203 -.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0 -1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997 .413l3.383 3.384A.705 .705 0 0 0 11 19.298z M16 9a5 5 0 0 1 0 6 M19.364 18.364a9 9 0 0 0 0 -12.728";
            case "prev": return "M0 0 m15 18-6-6 6-6";
            case "play": return "M5 5a2 2 0 0 1 3.008 -1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z";
            case "next": return "M0 0 m9 18 6-6-6-6";
            case "focus": return "M15 3h6v6 M0 0 m21 3-7 7 M0 0 m3 21 7-7 M9 21H3v-6";
            case "focus-in": return "M4 20l6-6 M5 14h5v5 M20 4l-6 6 M14 5v5h5";
            case "side": return "M8 3 4 7l4 4 M4 7h16 M0 0 m16 21 4-4-4-4 M20 17H4";
            case "desktop": return "M4,3 H20 Q22,3 22,5 V15 Q22,17 20,17 H4 Q2,17 2,15 V5 Q2,3 4,3 Z M8,21 L16,21 M12,17 L12,21";
            case "backspace": return "M10 5a2 2 0 0 0 -1.344 .519l-6.328 5.74a1 1 0 0 0 0 1.481l6.328 5.741A2 2 0 0 0 10 19h10a2 2 0 0 0 2 -2V7a2 2 0 0 0 -2 -2z M0 0 m12 9 6 6 M0 0 m18 9-6 6";
            case "up": return "M0 0 m5 12 7-7 7 7 M12 19V5";
            case "down": return "M12 5v14 M0 0 m19 12-7 7-7-7";
            case "windows": return "M5,3 H19 Q21,3 21,5 V19 Q21,21 19,21 H5 Q3,21 3,19 V5 Q3,3 5,3 Z M3 9h18 M9 21V9";
            case "upload": return "M12 3v12 M0 0 m17 8-5-5-5 5 M21 15v4a2 2 0 0 1 -2 2H5a2 2 0 0 1 -2 -2v-4";
            case "refresh": return "M20 4v6h-6 M20 10a8 8 0 1 0 0 5";
            case "download": return "M12 3v12 M7 10l5 5 5-5 M3 16v3a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-3";
            case "document": return "M6 3h8l4 4v14H6z M14 3v5h4 M9 13h6 M9 17h6";
            case "folder": return "M0 0 m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1 -1.95 1.5H4a2 2 0 0 1 -2 -2V5a2 2 0 0 1 2 -2h3.9a2 2 0 0 1 1.69 .9l.81 1.2a2 2 0 0 0 1.67 .9H18a2 2 0 0 1 2 2v2";
            case "send": return "M14.536 21.686a.5 .5 0 0 0 .937 -.024l6.5-19a.496 .496 0 0 0 -.635 -.635l-19 6.5a.5 .5 0 0 0 -.024 .937l7.93 3.18a2 2 0 0 1 1.112 1.11z M0 0 m21.854 2.147-10.94 10.939";
            case "star": return "M11.525 2.295a.53 .53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53 .53 0 0 1 .294 .904l-3.736 3.638a2.123 2.123 0 0 0 -.611 1.878l.882 5.14a.53 .53 0 0 1 -.771 .56l-4.618-2.428a2.122 2.122 0 0 0 -1.973 0L6.396 21.01a.53 .53 0 0 1 -.77 -.56l.881-5.139a2.122 2.122 0 0 0 -.611 -1.879L2.16 9.795a.53 .53 0 0 1 .294 -.906l5.165-.755a2.122 2.122 0 0 0 1.597 -1.16z";
            case "bolt": return "M15.914 4a1.5 1.5 0 0 0 -2.474 -1.561l-9 9A1.5 1.5 0 0 0 5.5 14h4.002a.5 .5 0 0 1 .471 .666L8.086 20a1.5 1.5 0 0 0 2.475 1.56l9-9A1.5 1.5 0 0 0 18.5 10h-3.997a.5 .5 0 0 1 -.472 -.667z";
            case "bell": return "M10.268 21a2 2 0 0 0 3.464 0 M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74 -1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326";
            case "grid": return "M4,3 H9 Q10,3 10,4 V9 Q10,10 9,10 H4 Q3,10 3,9 V4 Q3,3 4,3 Z M15,3 H20 Q21,3 21,4 V9 Q21,10 20,10 H15 Q14,10 14,9 V4 Q14,3 15,3 Z M15,14 H20 Q21,14 21,15 V20 Q21,21 20,21 H15 Q14,21 14,20 V15 Q14,14 15,14 Z M4,14 H9 Q10,14 10,15 V20 Q10,21 9,21 H4 Q3,21 3,20 V15 Q3,14 4,14 Z";
            case "monitor": return "M4,3 H20 Q22,3 22,5 V15 Q22,17 20,17 H4 Q2,17 2,15 V5 Q2,3 4,3 Z M8,21 L16,21 M12,17 L12,21";
            case "volume": return "M11 4.702a.705 .705 0 0 0 -1.203 -.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0 -1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997 .413l3.383 3.384A.705 .705 0 0 0 11 19.298z M16 9a5 5 0 0 1 0 6 M19.364 18.364a9 9 0 0 0 0 -12.728";
            case "search": return "M0 0 m21 21-4.34-4.34 M3,11 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0 Z";
            case "switch": return "M8 3 4 7l4 4 M4 7h16 M0 0 m16 21 4-4-4-4 M20 17H4";
            case "left": return "M0 0 m12 19-7-7 7-7 M19 12H5";
            case "right": return "M5 12h14 M0 0 m12 5 7 7-7 7";
            case "layers": return "M12.83 2.18a2 2 0 0 0 -1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0 -1.83z M2 12a1 1 0 0 0 .58 .91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12 M2 17a1 1 0 0 0 .58 .91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17";
            case "command": return "M15 6v12a3 3 0 1 0 3 -3H6a3 3 0 1 0 3 3V6a3 3 0 1 0 -3 3h12a3 3 0 1 0 -3 -3";
            case "gamepad": return "M6,11 L10,11 M8,9 L8,13 M15,12 L15.01,12 M18,10 L18.01,10 M17.32 5H6.68a4 4 0 0 0 -3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414 .586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3 -3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z";
            case "activity": return "M22 12h-2.48a2 2 0 0 0 -1.93 1.46l-2.35 8.36a.25 .25 0 0 1 -.48 0L9.24 2.18a.25 .25 0 0 0 -.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2";
            case "heart": return "M2 9.5a5.5 5.5 0 0 1 9.591 -3.676 .56 .56 0 0 0 .818 0A5.49 5.49 0 0 1 22 9.5c0 2.29-1.5 4-3 5.5l-5.492 5.313a2 2 0 0 1 -3 .019L5 15c-1.5-1.5-3-3.2-3-5.5";
            case "idea": return "M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5 M9 18h6 M10 22h4";
            default: return null;
        }
    }
}
