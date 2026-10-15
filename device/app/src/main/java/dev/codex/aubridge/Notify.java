package dev.codex.aubridge;

import android.app.Notification;
import android.content.ComponentName;
import android.content.Context;
import android.os.Bundle;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayDeque;
import java.util.HashMap;
import java.util.Map;

public final class Notify extends NotificationListenerService {
    private static volatile Notify instance;
    private final ArrayDeque<JSONObject> recent = new ArrayDeque<>(32);
    private final Map<String, StatusBarNotification> current = new HashMap<>();

    @Override
    public void onListenerConnected() {
        instance = this;
        try {
            StatusBarNotification[] active = getActiveNotifications();
            if (active != null) for (StatusBarNotification sbn : active) onNotificationPosted(sbn);
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onListenerDisconnected() {
        if (instance == this) instance = null;
    }

    @Override
    public void onNotificationPosted(StatusBarNotification sbn) {
        if (sbn == null) return;
        Notification n = sbn.getNotification();
        Bundle e = n == null ? null : n.extras;
        String title =
                e == null ? "" : String.valueOf(e.getCharSequence(Notification.EXTRA_TITLE, ""));
        String body =
                e == null ? "" : String.valueOf(e.getCharSequence(Notification.EXTRA_TEXT, ""));
        JSONObject item = new JSONObject();
        try {
            item.put("id", sbn.getKey());
            item.put("package", sbn.getPackageName());
            item.put("title", cut(title, 160));
            item.put("text", cut(body, 512));
            item.put("time", sbn.getPostTime());
            JSONArray actions = new JSONArray();
            if (n != null && n.actions != null)
                for (Notification.Action a : n.actions)
                    if (runnable(a)) actions.put(cut(a.title.toString(), 80));
            item.put("actions", actions);
            synchronized (recent) {
                current.put(sbn.getKey(), sbn);
                recent.removeIf(v -> sbn.getKey().equals(v.optString("id")));
                recent.addLast(item);
                while (recent.size() > 32) recent.removeFirst();
            }
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification sbn) {
        if (sbn == null) return;
        synchronized (recent) {
            current.remove(sbn.getKey());
            recent.removeIf(v -> sbn.getKey().equals(v.optString("id")));
        }
    }

    static boolean dismiss(String key) {
        Notify n = instance;
        if (n == null) return false;
        try {
            n.cancelNotification(key);
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    static boolean open(String key) {
        Notify n = instance;
        if (n == null) return false;
        try {
            StatusBarNotification sbn;
            synchronized (n.recent) {
                sbn = n.current.get(key);
            }
            if (sbn == null || sbn.getNotification().contentIntent == null) return false;
            sbn.getNotification().contentIntent.send();
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    /** Runs the action titled by the label after \u0001, or the only action when none is given. */
    static boolean action(String request) {
        Notify n = instance;
        if (n == null) return false;
        int split = request.lastIndexOf('\u0001');
        String key = split < 0 ? request : request.substring(0, split);
        String label = split < 0 ? null : cut(request.substring(split + 1), 80);
        try {
            StatusBarNotification sbn;
            synchronized (n.recent) {
                sbn = n.current.get(key);
            }
            if (sbn == null || sbn.getNotification().actions == null) return false;
            Notification.Action chosen = null;
            int runnable = 0;
            for (Notification.Action action : sbn.getNotification().actions) {
                if (!runnable(action)) continue;
                runnable++;
                if (label == null
                        ? chosen == null
                        : label.equalsIgnoreCase(cut(action.title.toString(), 80))) chosen = action;
            }
            if (chosen == null || (label == null && runnable > 1)) return false;
            chosen.actionIntent.send();
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    static JSONObject read(Context context) throws Exception {
        String enabled =
                Settings.Secure.getString(
                        context.getContentResolver(), "enabled_notification_listeners");
        boolean allowed =
                enabled != null
                        && enabled.contains(
                                new ComponentName(context, Notify.class).flattenToString());
        JSONArray out = new JSONArray();
        if (instance != null)
            synchronized (instance.recent) {
                for (JSONObject item : instance.recent) out.put(item);
            }
        return new JSONObject()
                .put("enabled", allowed)
                .put("active", instance != null)
                .put("items", out)
                .put("truncated", out.length() >= 32);
    }

    /** Actions that need typed input (Reply) cannot be run from here, so they are not offered. */
    private static boolean runnable(Notification.Action a) {
        return a != null
                && a.actionIntent != null
                && a.title != null
                && a.getRemoteInputs() == null;
    }

    private static String cut(String s, int max) {
        return s.length() <= max ? s : s.substring(0, max);
    }
}
