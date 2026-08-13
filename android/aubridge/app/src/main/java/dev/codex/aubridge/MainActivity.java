package dev.codex.aubridge;

import android.Manifest;
import android.app.Activity;
import android.content.ComponentName;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.text.TextUtils;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/** Small, production-facing setup surface. Capability requests are explicit and independent. */
public final class MainActivity extends Activity {
    private LinearLayout capabilities;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        buildView();
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (capabilities != null) refreshCapabilities();
    }

    private void buildView() {
        ScrollView scroll = new ScrollView(this);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int padding = 24;
        layout.setPadding(padding, padding, padding, padding);

        TextView heading = new TextView(this);
        heading.setText("ANDROID USE\nLocal-first Android control");
        heading.setTextSize(24);
        layout.addView(heading);

        TextView explanation = new TextView(this);
        explanation.setText("This screen only enables capabilities you choose. The local bridge has no Internet permission; your computer connects through an authenticated ADB forward.");
        explanation.setPadding(0, 16, 0, 16);
        layout.addView(explanation);

        layout.addView(button("Start local bridge", view -> startBridge()));
        layout.addView(button("Stop local bridge", view -> stopService(new Intent(this, AuBridgeService.class))));
        layout.addView(button("Semantic control settings", view -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))));
        layout.addView(button("Notification access settings", view -> startActivity(new Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"))));

        TextView paired = new TextView(this);
        paired.setText("\nPaired computer\nLocal ADB enrollment is managed by android-use on the computer.\nRemote access\nOFF — no network broker is enabled in this local helper.");
        paired.setPadding(0, 12, 0, 12);
        layout.addView(paired);

        TextView title = new TextView(this);
        title.setText("Capabilities");
        title.setTextSize(18);
        layout.addView(title);
        capabilities = new LinearLayout(this);
        capabilities.setOrientation(LinearLayout.VERTICAL);
        layout.addView(capabilities);

        TextView audit = new TextView(this);
        audit.setText("\nThe bridge stops when you stop it. Android's system settings remain the authority for accessibility, notification, and runtime permissions.");
        audit.setPadding(0, 12, 0, 12);
        layout.addView(audit);

        scroll.addView(layout);
        setContentView(scroll);
        refreshCapabilities();
    }

    private void refreshCapabilities() {
        capabilities.removeAllViews();
        addCapability("Local bridge", BridgeService.isRunning(), null);
        addCapability("Semantic control", accessibilityEnabled(), () -> startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));
        addCapability("Notification access", notificationAccessEnabled(), () -> startActivity(new Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")));
        addCapability("Camera", granted(Manifest.permission.CAMERA), () -> requestOne(Manifest.permission.CAMERA));
        addCapability("Microphone", granted(Manifest.permission.RECORD_AUDIO), () -> requestOne(Manifest.permission.RECORD_AUDIO));
        if (Build.VERSION.SDK_INT >= 33) {
            addCapability("Notifications", granted(Manifest.permission.POST_NOTIFICATIONS), () -> requestOne(Manifest.permission.POST_NOTIFICATIONS));
        }
    }

    private void addCapability(String name, boolean enabled, Runnable action) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        TextView state = new TextView(this);
        state.setText((enabled ? "✓ " : "○ ") + name);
        state.setTextSize(17);
        state.setPadding(0, 8, 8, 8);
        row.addView(state, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        if (!enabled && action != null) {
            Button enable = new Button(this);
            enable.setText("Enable");
            enable.setOnClickListener(view -> action.run());
            row.addView(enable);
        }
        capabilities.addView(row);
    }

    private Button button(String text, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(text);
        button.setOnClickListener(listener);
        return button;
    }

    private void startBridge() {
        startForegroundService(new Intent(this, AuBridgeService.class));
        refreshCapabilities();
    }

    private void requestOne(String permission) {
        if (!granted(permission)) requestPermissions(new String[]{permission}, permission.hashCode() & 0x7fff);
    }

    private boolean granted(String permission) {
        return checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }

    private boolean accessibilityEnabled() {
        String enabled = Settings.Secure.getString(getContentResolver(), Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES);
        if (TextUtils.isEmpty(enabled)) return false;
        ComponentName component = new ComponentName(this, AubridgeAccessibilityService.class);
        return enabled.contains(component.flattenToString());
    }

    private boolean notificationAccessEnabled() {
        String enabled = Settings.Secure.getString(getContentResolver(), "enabled_notification_listeners");
        return !TextUtils.isEmpty(enabled) && enabled.contains(getPackageName());
    }
}
