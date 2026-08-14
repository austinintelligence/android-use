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
    @Override public void onCreate() {
        super.onCreate();
        NotificationManager n=getSystemService(NotificationManager.class);
        n.createNotificationChannel(new NotificationChannel("bridge","AU Bridge",NotificationManager.IMPORTANCE_LOW));
        Notification note=new Notification.Builder(this,"bridge").setSmallIcon(android.R.drawable.stat_sys_data_bluetooth).setContentTitle("AU Bridge").setContentText("Authenticated local Android control").build();
        startForeground(31,note);
        instance=this;
        bridge=new Bridge(this);
        bridge.start();
    }
    @Override public int onStartCommand(Intent intent,int flags,int id){return START_STICKY;}
    @Override public void onDestroy(){if(instance==this)instance=null;if(bridge!=null)bridge.close();super.onDestroy();}
    @Override public IBinder onBind(Intent intent){return null;}

    static void projectionStarted() {
        BridgeService service=instance;
        if(service==null||Build.VERSION.SDK_INT<29)return;
        Notification note=new Notification.Builder(service,"bridge").setSmallIcon(android.R.drawable.stat_sys_data_bluetooth).setContentTitle("AU Bridge").setContentText("Screen recording is active").build();
        int type=ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION;
        if(Build.VERSION.SDK_INT>=34)type|=ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE;
        service.startForeground(31,note,type);
    }
}
