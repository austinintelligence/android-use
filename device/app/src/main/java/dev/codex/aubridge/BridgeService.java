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
        if (Build.VERSION.SDK_INT >= 29)
            promote(ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION, "Screen recording");
    }

    /** Android 14+ needs the capture's own service type; older releases already claim all. */
    static void capturing(int type, String what) {
        if (Build.VERSION.SDK_INT >= 34) promote(type, what);
    }

    /** Drops the capture type again once the capture is over. */
    static void settle() {
        if (Build.VERSION.SDK_INT >= 34) promote(0, null);
    }

    private static void promote(int type, String what) {
        BridgeService service = instance;
        if (service == null) return;
        Notification.Builder note =
                new Notification.Builder(service, "bridge")
                        .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                        .setContentTitle("Android-Use")
                        .setContentText(
                                what == null
                                        ? "Authenticated local Android control"
                                        : what + " is active");
        if (Build.VERSION.SDK_INT >= 34) type |= ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        try {
            service.startForeground(31, note.build(), type);
        } catch (RuntimeException e) {
            // The caller reports the capture's own permission or background failure.
        }
    }
}
