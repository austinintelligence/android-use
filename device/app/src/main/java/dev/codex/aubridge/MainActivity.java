package dev.codex.aubridge;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.media.projection.MediaProjectionManager;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.Settings;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

public final class MainActivity extends Activity {
    private static final int RECORD_REQUEST = 41;
    private TextView grants;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setPadding(48, 64, 48, 48);
        TextView title = new TextView(this);
        title.setText(
                "Android-Use\n\n"
                    + "Turn on its accessibility service, then use the Android-Use agent on your"
                    + " computer.");
        title.setTextSize(20);
        grants = new TextView(this);
        Button open = new Button(this);
        open.setText("Open accessibility settings");
        open.setOnClickListener(
                v -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));
        Button privacy = new Button(this);
        privacy.setText("Grant camera, microphone, location, and notifications");
        privacy.setOnClickListener(
                v ->
                        requestPermissions(
                                new String[] {
                                    Manifest.permission.CAMERA,
                                    Manifest.permission.RECORD_AUDIO,
                                    Manifest.permission.ACCESS_COARSE_LOCATION,
                                    Manifest.permission.ACCESS_FINE_LOCATION,
                                    Manifest.permission.POST_NOTIFICATIONS
                                },
                                40));
        Button notifications = new Button(this);
        notifications.setText("Open notification access settings");
        notifications.setOnClickListener(
                v ->
                        startActivity(
                                new Intent(
                                        "android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")));
        Button recording = new Button(this);
        recording.setText("Allow screen recording");
        recording.setOnClickListener(
                v -> {
                    MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
                    if (manager != null)
                        startActivityForResult(manager.createScreenCaptureIntent(), RECORD_REQUEST);
                });
        page.addView(title);
        page.addView(grants);
        page.addView(open);
        page.addView(privacy);
        page.addView(notifications);
        page.addView(recording);
        setContentView(page);
    }

    @Override
    protected void onResume() {
        super.onResume();
        grants.setText(status());
    }

    private static String yes(boolean on) {
        return on ? "on" : "off";
    }

    /** Each grant the host depends on, the helper version, and when the host last called. */
    private String status() {
        String version = "?";
        try {
            version = getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (PackageManager.NameNotFoundException ignored) {
        }
        boolean access = false, listener = false, battery = false;
        try {
            access = Ui.get() != null;
            listener = Notify.read(this).optBoolean("enabled");
            battery = Bridge.env(this).optBoolean("battery_exempt");
        } catch (Exception ignored) {
        }
        boolean posts =
                Build.VERSION.SDK_INT < 33
                        || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                                == PackageManager.PERMISSION_GRANTED;
        long ago = (SystemClock.elapsedRealtime() - Bridge.lastContact) / 1000;
        return "Helper "
                + version
                + (BridgeService.stopped(this) ? " (stopped)" : "")
                + "\nAccessibility: "
                + yes(access)
                + "\nNotification access: "
                + yes(listener)
                + "\nNotification permission: "
                + yes(posts)
                + "\nBattery optimization exemption: "
                + yes(battery)
                + "\nLast contact from computer: "
                + (Bridge.lastContact == 0 ? "never" : ago + " s ago");
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == RECORD_REQUEST && result == RESULT_OK && data != null)
            Projection.set(result, data);
    }
}
