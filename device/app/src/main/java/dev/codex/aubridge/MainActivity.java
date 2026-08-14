package dev.codex.aubridge;

import android.app.Activity;
import android.Manifest;
import android.content.Intent;
import android.media.projection.MediaProjectionManager;
import android.os.Bundle;
import android.provider.Settings;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

public final class MainActivity extends Activity {
    private static final int RECORD_REQUEST = 41;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setPadding(48, 64, 48, 48);
        TextView title = new TextView(this);
        title.setText("AU Bridge\n\nEnable accessibility control, then use the paired desktop CLI.");
        title.setTextSize(20);
        Button open = new Button(this);
        open.setText("Open accessibility settings");
        open.setOnClickListener(v -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));
        Button privacy = new Button(this);
        privacy.setText("Grant camera, microphone, and location");
        privacy.setOnClickListener(v -> requestPermissions(new String[]{
                Manifest.permission.CAMERA,
                Manifest.permission.RECORD_AUDIO,
                Manifest.permission.ACCESS_COARSE_LOCATION,
                Manifest.permission.ACCESS_FINE_LOCATION
        }, 40));
        Button notifications = new Button(this);
        notifications.setText("Open notification access settings");
        notifications.setOnClickListener(v -> startActivity(new Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")));
        Button recording = new Button(this);
        recording.setText("Allow screen recording");
        recording.setOnClickListener(v -> {
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            if (manager != null) startActivityForResult(manager.createScreenCaptureIntent(), RECORD_REQUEST);
        });
        page.addView(title);
        page.addView(open);
        page.addView(privacy);
        page.addView(notifications);
        page.addView(recording);
        setContentView(page);
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == RECORD_REQUEST && result == RESULT_OK && data != null) Projection.set(result, data);
    }
}
