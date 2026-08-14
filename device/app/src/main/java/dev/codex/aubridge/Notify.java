package dev.codex.aubridge;

import android.app.Notification;
import android.content.ComponentName;
import android.content.Context;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.os.Bundle;
import org.json.JSONArray;
import org.json.JSONObject;
import java.util.ArrayDeque;
import java.util.HashMap;
import java.util.Map;

public final class Notify extends NotificationListenerService {
    private static volatile Notify instance;
    private final ArrayDeque<JSONObject> recent=new ArrayDeque<>(32);
    private final Map<String,StatusBarNotification> current=new HashMap<>();
    @Override public void onListenerConnected(){instance=this;}
    @Override public void onListenerDisconnected(){if(instance==this)instance=null;}
    @Override public void onNotificationPosted(StatusBarNotification sbn){if(sbn==null)return;Notification n=sbn.getNotification();Bundle e=n==null?null:n.extras;String title=e==null?"":String.valueOf(e.getCharSequence(Notification.EXTRA_TITLE,""));String body=e==null?"":String.valueOf(e.getCharSequence(Notification.EXTRA_TEXT,""));JSONObject item=new JSONObject();try{item.put("id",sbn.getKey());item.put("package",sbn.getPackageName());item.put("title",cut(title,160));item.put("text",cut(body,512));item.put("time",sbn.getPostTime());synchronized(recent){current.put(sbn.getKey(),sbn);recent.removeIf(v->sbn.getKey().equals(v.optString("id")));recent.addLast(item);while(recent.size()>32)recent.removeFirst();}}catch(Exception ignored){}}
    @Override public void onNotificationRemoved(StatusBarNotification sbn){if(sbn==null)return;synchronized(recent){current.remove(sbn.getKey());recent.removeIf(v->sbn.getKey().equals(v.optString("id")));}}
    static boolean dismiss(String key){Notify n=instance;if(n==null)return false;try{n.cancelNotification(key);return true;}catch(Exception ignored){return false;}}
    static boolean open(String key){Notify n=instance;if(n==null)return false;try{StatusBarNotification sbn; synchronized(n.recent){sbn=n.current.get(key);}if(sbn==null||sbn.getNotification().contentIntent==null)return false;sbn.getNotification().contentIntent.send();return true;}catch(Exception ignored){return false;}}
    static boolean action(String key){Notify n=instance;if(n==null)return false;try{StatusBarNotification sbn; synchronized(n.recent){sbn=n.current.get(key);}if(sbn==null||sbn.getNotification().actions==null)return false;for(Notification.Action action:sbn.getNotification().actions){if(action!=null&&action.actionIntent!=null){action.actionIntent.send();return true;}}return false;}catch(Exception ignored){return false;}}
    static JSONObject read(Context context)throws Exception{String enabled=Settings.Secure.getString(context.getContentResolver(),"enabled_notification_listeners");boolean allowed=enabled!=null&&enabled.contains(new ComponentName(context,Notify.class).flattenToString());JSONArray out=new JSONArray();if(instance!=null)synchronized(instance.recent){for(JSONObject item:instance.recent)out.put(item);}return new JSONObject().put("enabled",allowed).put("active",instance!=null).put("items",out).put("truncated",out.length()>=32);}
    private static String cut(String s,int max){return s.length()<=max?s:s.substring(0,max);}
}
