package dev.codex.aubridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;

public final class BridgeService extends Service {
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
        bridge = new Bridge(this);
        bridge.start();
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

    static void projectionStarted() {
        promote("Screen recording is active", ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);
    }

    /** Puts the service back to its idle type and notification once a recording is over. */
    static void projectionStopped() {
        promote("Authenticated local Android control", 0);
    }

    private static void promote(String text, int projection) {
        BridgeService service = instance;
        if (service == null || Build.VERSION.SDK_INT < 29) return;
        Notification note =
                new Notification.Builder(service, "bridge")
                        .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                        .setContentTitle("Android-Use")
                        .setContentText(text)
                        .build();
        // Before Android 14 the service claims every manifest type; narrowing it to the
        // projection would drop camera, microphone and location until it restarts.
        int type = ServiceInfo.FOREGROUND_SERVICE_TYPE_MANIFEST;
        if (Build.VERSION.SDK_INT >= 34)
            type = projection | ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        try {
            service.startForeground(31, note, type);
        } catch (RuntimeException e) {
            // The recording reports its own failure.
        }
    }
}
