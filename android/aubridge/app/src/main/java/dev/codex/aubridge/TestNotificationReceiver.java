package dev.codex.aubridge;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

public final class TestNotificationReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        context.getSharedPreferences("au_test", Context.MODE_PRIVATE)
                .edit()
                .putBoolean("notification_action_handled", true)
                .apply();
    }
}
