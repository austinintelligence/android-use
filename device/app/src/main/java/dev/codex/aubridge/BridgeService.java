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

public final class BridgeService extends Service {
    static final String STOP = "dev.codex.aubridge.action.STOP";
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
        bridge = new Bridge(this);
        bridge.start();
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
        context.getSharedPreferences("au", MODE_PRIVATE).edit().putBoolean("stopped", value).apply();
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
        BridgeService service = instance;
        if (service == null || Build.VERSION.SDK_INT < 29) return;
        Notification note =
                new Notification.Builder(service, "bridge")
                        .setSmallIcon(R.drawable.ic_notification)
                        .setContentTitle("Android-Use")
                        .setContentText("Screen recording is active")
                        .build();
        int type = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION;
        if (Build.VERSION.SDK_INT >= 34) type |= ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        service.startForeground(31, note, type);
    }
}
