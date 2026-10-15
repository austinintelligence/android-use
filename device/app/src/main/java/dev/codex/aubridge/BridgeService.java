package dev.codex.aubridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

public final class BridgeService extends Service {
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
                        .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                        .setContentTitle("Android-Use")
                        .setContentText("Authenticated local Android control")
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
        return START_STICKY;
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
                        .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
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
