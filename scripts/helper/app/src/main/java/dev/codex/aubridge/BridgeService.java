package dev.codex.aubridge;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.IBinder;

import java.io.File;
import java.io.FileOutputStream;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.concurrent.atomic.AtomicLong;

/** Shared implementation for the current AU foreground bridge component. */
public class BridgeService extends Service {
    private final AtomicLong heartbeatAtMs = new AtomicLong(0L);
    private BridgeServer server;

    @Override
    public void onCreate() {
        super.onCreate();
        String token = loadOrCreateToken();
        server = new BridgeServer(this, token);
        server.start();
        createChannel();
        Notification notification = new Notification.Builder(this, "au-bridge")
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle("AU Bridge active")
                .setContentText("Authenticated local ADB-forwarded control only")
                .build();
        startForeground(4101, notification);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        heartbeatAtMs.set(0L);
        if (server != null) {
            server.close();
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    public void heartbeat() {
        heartbeatAtMs.set(System.currentTimeMillis());
    }

    public boolean heartbeatFresh() {
        long value = heartbeatAtMs.get();
        return value > 0 && System.currentTimeMillis() - value <= 3_000L;
    }

    private String loadOrCreateToken() {
        File tokenFile = new File(getFilesDir(), "bridge_token");
        try {
            if (tokenFile.isFile()) {
                String existing = new String(
                        java.nio.file.Files.readAllBytes(tokenFile.toPath()),
                        java.nio.charset.StandardCharsets.UTF_8).trim();
                if (existing.length() >= 32) {
                    return existing;
                }
            }
            byte[] random = new byte[32];
            new SecureRandom().nextBytes(random);
            String token = Base64.getUrlEncoder().withoutPadding().encodeToString(random);
            try (FileOutputStream output = openFileOutput("bridge_token", MODE_PRIVATE)) {
                output.write(token.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                output.getFD().sync();
            }
            return token;
        } catch (Exception error) {
            throw new IllegalStateException("Cannot create private bridge token", error);
        }
    }

    private void createChannel() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("au-bridge", "AU Bridge", NotificationManager.IMPORTANCE_LOW));
    }
}
