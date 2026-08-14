package dev.codex.aubridge;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

/** Debug-only deterministic UI fixture; never shipped in the production APK. */
public final class TestActivity extends Activity {
    private TextView state;

    @Override
    protected void onCreate(Bundle bundle) {
        super.onCreate(bundle);
        ScrollView scroll = new ScrollView(this);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(24, 24, 24, 24);
        scroll.addView(layout);

        TextView title = label("AU deterministic test activity");
        title.setContentDescription("AU test title");
        layout.addView(title);

        EditText input = new EditText(this);
        input.setHint("Editable test text");
        input.setContentDescription("AU editable text");
        input.setSingleLine(false);
        layout.addView(input);

        Button tap = new Button(this);
        tap.setText("Tap target");
        tap.setContentDescription("AU tap target");
        tap.setOnClickListener(view -> state.setText("Tapped"));
        layout.addView(tap);

        Button longPress = new Button(this);
        longPress.setText("Long press target");
        longPress.setContentDescription("AU long press target");
        longPress.setOnLongClickListener(view -> {
            state.setText("Long pressed");
            return true;
        });
        layout.addView(longPress);

        Switch toggle = new Switch(this);
        toggle.setText("Toggle target");
        toggle.setContentDescription("AU toggle target");
        toggle.setOnCheckedChangeListener((button, checked) -> state.setText("Toggle=" + checked));
        layout.addView(toggle);

        Button dialog = new Button(this);
        dialog.setText("Open deterministic dialog");
        dialog.setContentDescription("AU dialog target");
        dialog.setOnClickListener(view -> new AlertDialog.Builder(this)
                .setTitle("AU dialog")
                .setMessage("Deterministic confirmation")
                .setNegativeButton("Cancel", null)
                .setPositiveButton("Allow", (ignored, which) -> state.setText("Dialog allowed"))
                .show());
        layout.addView(dialog);

        Button notification = new Button(this);
        notification.setText("Post deterministic notification");
        notification.setContentDescription("AU notification target");
        notification.setOnClickListener(view -> postNotification());
        layout.addView(notification);

        state = label("Ready");
        state.setContentDescription("AU state");
        layout.addView(state);

        for (int index = 1; index <= 40; index++) {
            TextView item = label("Scroll item " + index);
            item.setContentDescription("AU scroll item " + index);
            layout.addView(item);
        }
        setContentView(scroll);
    }

    private TextView label(String text) {
        TextView view = new TextView(this);
        view.setText(text);
        view.setTextSize(18);
        view.setPadding(0, 12, 0, 12);
        return view;
    }

    private void postNotification() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        String channel = "au-test";
        manager.createNotificationChannel(new NotificationChannel(channel, "AU test", NotificationManager.IMPORTANCE_DEFAULT));
        android.app.Notification notification = new android.app.Notification.Builder(this, channel)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle("AU deterministic notification")
                .setContentText("Notification listener validation")
                .setContentIntent(PendingIntent.getActivity(
                        this,
                        7003,
                        new Intent(this, TestActivity.class),
                        PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE))
                .addAction(new android.app.Notification.Action.Builder(
                        android.graphics.drawable.Icon.createWithResource(this, android.R.drawable.ic_menu_view),
                        "Mark handled",
                        PendingIntent.getBroadcast(this, 7002,
                                new Intent(this, TestNotificationReceiver.class),
                                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE))
                        .build())
                .build();
        manager.notify(7001, notification);
        state.setText("Notification posted");
    }
}
