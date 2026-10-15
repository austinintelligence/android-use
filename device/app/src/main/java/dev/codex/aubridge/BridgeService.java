package dev.codex.aubridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

public final class BridgeService extends Service {
    static final String STOP = "dev.codex.aubridge.action.STOP";
    private static final Safety.Held HELD = new Safety.Held();
    private static volatile BridgeService instance;
    private Bridge bridge;

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager n = getSystemService(NotificationManager.class);
        n.createNotificationChannel(
                new NotificationChannel(
                        "bridge", "Android-Use", NotificationManager.IMPORTANCE_LOW));
        Notification note =
                new Notification.Builder(this, "bridge")
                        .setSmallIcon(R.drawable.ic_notification)
                        .setContentTitle("Android-Use")
                        .setContentText("Authenticated local Android control")
                        .addAction(
                                new Notification.Action.Builder(
                                                null,
                                                "Stop",
                                                PendingIntent.getService(
                                                        this,
                                                        0,
                                                        new Intent(this, BridgeService.class)
                                                                .setAction(STOP),
                                                        PendingIntent.FLAG_IMMUTABLE))
                                        .build())
                        .build();
        // Without a type, startForeground claims every manifest type, and claiming
        // mediaProjection before consent throws on API 34+. Projection adds it later.
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(31, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(31, note);
        }
        instance = this;
        bridge = new Bridge(this, this::bindFailed);
        bridge.start();
    }

    private void bindFailed() {
        Log.e("AndroidUse", "cannot listen: another process holds the helper socket");
        getSystemService(NotificationManager.class)
                .notify(
                        32,
                        new Notification.Builder(this, "bridge")
                                .setSmallIcon(android.R.drawable.stat_sys_warning)
                                .setContentTitle("Android-Use cannot start")
                                .setContentText("Another process holds the helper socket")
                                .build());
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int id) {
        boolean stop = intent != null && STOP.equals(intent.getAction());
        // `au setup` passes resume; any other start (a sticky restart, the host
        // reconnecting) keeps a stopped helper stopped.
        if (intent != null && intent.getBooleanExtra("resume", false)) setStopped(this, false);
        else if (stop) setStopped(this, true);
        if (stopped(this)) stopForeground(STOP_FOREGROUND_REMOVE);
        return START_STICKY;
    }

    /** Stopped means the notification is gone and the bridge refuses everything but `env`. */
    static boolean stopped(Context context) {
        return context.getSharedPreferences("au", MODE_PRIVATE).getBoolean("stopped", false);
    }

    private static void setStopped(Context context, boolean value) {
        context.getSharedPreferences("au", MODE_PRIVATE)
                .edit()
                .putBoolean("stopped", value)
                .apply();
    }

    @Override
    public void onDestroy() {
        if (instance == this) instance = null;
        if (bridge != null) bridge.close();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    static void begin(int type) {
        update(type, true);
    }

    static void end(int type) {
        update(type, false);
    }

    /** Keeps the union of the types in use, so one capture ending cannot drop another's. */
    private static synchronized void update(int type, boolean on) {
        BridgeService service = instance;
        if (service == null || Build.VERSION.SDK_INT < 29) return;
        // Before Android 14 the service claims every manifest type; narrowing it would drop
        // camera, microphone and location until it restarts. Only the notification changes.
        boolean modern = Build.VERSION.SDK_INT >= 34;
        if (!modern && type != ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION) return;
        int mask = on ? HELD.add(type) : HELD.remove(type);
        Notification note =
                new Notification.Builder(service, "bridge")
                        .setSmallIcon(R.drawable.ic_notification)
                        .setContentTitle("Android-Use")
                        .setContentText(
                                mask == 0
                                        ? "Authenticated local Android control"
                                        : "Capture is active")
                        .build();
        try {
            service.startForeground(
                    31,
                    note,
                    modern
                            ? mask | ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
                            : ServiceInfo.FOREGROUND_SERVICE_TYPE_MANIFEST);
        } catch (RuntimeException e) {
            // The capture reports its own permission or background failure.
        }
    }
}
