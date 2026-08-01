package dev.codex.aubridge;

import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

public final class AubridgeNotificationListener extends NotificationListenerService {
    private static volatile AubridgeNotificationListener instance;
    private final List<JSONObject> events = new ArrayList<>();

    static AubridgeNotificationListener current() {
        return instance;
    }

    @Override
    public void onListenerConnected() {
        instance = this;
    }

    @Override
    public void onDestroy() {
        if (instance == this) instance = null;
        super.onDestroy();
    }

    @Override
    public void onNotificationPosted(StatusBarNotification notification) {
        recordEvent("posted", notification);
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification notification) {
        recordEvent("removed", notification);
    }

    synchronized JSONObject handle(String operation, JSONObject args) throws Exception {
        if (operation.equals("notification.ls")) {
            JSONArray notifications = new JSONArray();
            StatusBarNotification[] active = getActiveNotifications();
            int limit = Math.min(active.length, 256);
            for (int index = 0; index < limit; index++) {
                StatusBarNotification notification = active[index];
                notifications.put(new JSONObject()
                        .put("key", notification.getKey())
                        .put("package", notification.getPackageName())
                        .put("title", bounded(String.valueOf(notification.getNotification().extras.getCharSequence("android.title", "")), 512))
                        .put("text", bounded(String.valueOf(notification.getNotification().extras.getCharSequence("android.text", "")), 2_000))
                        .put("actions", notification.getNotification().actions == null ? 0 : notification.getNotification().actions.length));
            }
            return new JSONObject().put("notifications", notifications).put("truncated", active.length > limit);
        }
        if (operation.equals("notification.watch")) {
            JSONArray changed = new JSONArray();
            for (JSONObject event : events) {
                changed.put(new JSONObject(event.toString()));
            }
            events.clear();
            return new JSONObject().put("events", changed);
        }
        org.json.JSONArray values = args.optJSONArray("args");
        String key = values == null ? "" : values.optString(0, "");
        if (operation.equals("notification.dismiss")) {
            cancelNotification(key);
            return new JSONObject().put("dismissed", key);
        }
        if (operation.equals("notification.open")) {
            for (StatusBarNotification notification : getActiveNotifications()) {
                if (notification.getKey().equals(key) && notification.getNotification().contentIntent != null) {
                    notification.getNotification().contentIntent.send();
                    return new JSONObject().put("opened", key);
                }
            }
            throw new BridgeServer.BridgeError("E_NOTIFICATION", "notification not found or not actionable");
        }
        if (operation.equals("notification.action")) {
            if (values == null || values.length() < 2) {
                throw new BridgeServer.BridgeError("E_ARGS", "notif action KEY INDEX");
            }
            int index;
            try {
                index = Integer.parseInt(values.optString(1));
            } catch (NumberFormatException error) {
                throw new BridgeServer.BridgeError("E_ARGS", "notification action index must be numeric");
            }
            for (StatusBarNotification notification : getActiveNotifications()) {
                if (!notification.getKey().equals(key)) continue;
                android.app.Notification.Action[] actions = notification.getNotification().actions;
                if (actions == null || index < 0 || index >= actions.length || actions[index] == null || actions[index].actionIntent == null) {
                    throw new BridgeServer.BridgeError("E_NOTIFICATION", "notification action is unavailable");
                }
                actions[index].actionIntent.send();
                return new JSONObject().put("acted", key).put("index", index);
            }
            throw new BridgeServer.BridgeError("E_NOTIFICATION", "notification not found");
        }
        throw new BridgeServer.BridgeError("E_ARGS", "unknown notification operation " + operation);
    }

    private synchronized void recordEvent(String type, StatusBarNotification notification) {
        try {
            events.add(new JSONObject()
                    .put("event", type)
                    .put("key", notification.getKey())
                    .put("package", notification.getPackageName())
                    .put("title", bounded(String.valueOf(notification.getNotification().extras.getCharSequence("android.title", "")), 512)));
            if (events.size() > 128) events.remove(0);
        } catch (Exception ignored) {
        }
    }

    private static String bounded(String value, int limit) {
        return value.length() <= limit ? value : value.substring(0, limit);
    }
}
