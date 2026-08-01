package dev.codex.aubridge;

import android.app.Activity;
import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

public final class MainActivity extends Activity {
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int padding = 24;
        layout.setPadding(padding, padding, padding, padding);

        TextView heading = new TextView(this);
        heading.setText("AU Bridge\nLocal authenticated Android control. No network listener.");
        layout.addView(heading);
        layout.addView(button("Start local bridge", view -> startForegroundService(new Intent(this, AuBridgeService.class))));
        layout.addView(button("Grant camera, microphone, location, and notification permissions", view -> requestRuntimePermissions()));
        layout.addView(button("Accessibility settings", view -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))));
        layout.addView(button("Notification access", view -> startActivity(new Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"))));
        layout.addView(button("Open deterministic test activity", view -> startActivity(new Intent(this, TestActivity.class))));
        setContentView(layout);
    }

    private Button button(String text, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(text);
        button.setOnClickListener(listener);
        return button;
    }

    private void requestRuntimePermissions() {
        String[] permissions;
        if (Build.VERSION.SDK_INT >= 33) {
            permissions = new String[]{
                    Manifest.permission.CAMERA,
                    Manifest.permission.RECORD_AUDIO,
                    Manifest.permission.ACCESS_FINE_LOCATION,
                    Manifest.permission.ACCESS_COARSE_LOCATION,
                    Manifest.permission.POST_NOTIFICATIONS
            };
        } else {
            permissions = new String[]{
                    Manifest.permission.CAMERA,
                    Manifest.permission.RECORD_AUDIO,
                    Manifest.permission.ACCESS_FINE_LOCATION,
                    Manifest.permission.ACCESS_COARSE_LOCATION
            };
        }
        boolean needed = false;
        for (String permission : permissions) {
            if (checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
                needed = true;
                break;
            }
        }
        if (needed) {
            requestPermissions(permissions, 4102);
        }
    }
}
