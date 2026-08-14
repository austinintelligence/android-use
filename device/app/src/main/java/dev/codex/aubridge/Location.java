package dev.codex.aubridge;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Build;
import android.os.Looper;
import org.json.JSONObject;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

final class Location {
    private Location() {}
    static JSONObject read(Context context) throws Exception {
        boolean permission=context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)==PackageManager.PERMISSION_GRANTED||context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION)==PackageManager.PERMISSION_GRANTED;
        JSONObject out=new JSONObject().put("permission",permission);
        if(!permission)return out.put("available",false);
        LocationManager manager=context.getSystemService(LocationManager.class);if(manager==null)return out.put("available",false);
        List<String> providers=manager.getProviders(true);AtomicReference<android.location.Location> fresh=new AtomicReference<>();CountDownLatch done=new CountDownLatch(1);
        LocationListener listener=new LocationListener(){@Override public void onLocationChanged(android.location.Location item){if(item!=null){fresh.compareAndSet(null,item);done.countDown();}}};
        try{
            for(String provider:providers){
                try{if(Build.VERSION.SDK_INT>=31)manager.requestLocationUpdates(provider,0L,0f,context.getMainExecutor(),listener);
                else manager.requestLocationUpdates(provider,0L,0f,listener,Looper.getMainLooper());}catch(SecurityException ignored){}
            }
            done.await(5,TimeUnit.SECONDS);
        }catch(SecurityException ignored){}finally{try{manager.removeUpdates(listener);}catch(SecurityException ignored){}}
        android.location.Location best=fresh.get();
        if(best==null)for(String provider:providers){try{android.location.Location item=manager.getLastKnownLocation(provider);if(item!=null&&(best==null||item.getTime()>best.getTime()))best=item;}catch(SecurityException ignored){}}
        if(best==null)return out.put("available",false);
        long age=Math.max(0,System.currentTimeMillis()-best.getTime());
        return out.put("available",true).put("current",fresh.get()!=null).put("lat",best.getLatitude()).put("lon",best.getLongitude()).put("acc",best.hasAccuracy()?best.getAccuracy():-1).put("age_ms",Math.min(age,86_400_000L)).put("provider",best.getProvider()==null?"":best.getProvider());
    }
}
