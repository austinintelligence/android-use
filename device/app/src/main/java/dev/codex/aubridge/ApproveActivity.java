package dev.codex.aubridge;

import android.app.Activity;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

/**
 * Full-screen "Allow once / Deny" prompt for an approval the agent asked for. It shows the exact
 * command that will run, so the person is never asked about one thing and given another.
 */
public final class ApproveActivity extends Activity {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable watch =
            new Runnable() {
                @Override
                public void run() {
                    if (!Approval.waiting(SystemClock.elapsedRealtime())) {
                        finish();
                        return;
                    }
                    handler.postDelayed(this, 500);
                }
            };

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // Not shown over the lock screen: only someone who can unlock the phone may answer.
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setPadding(48, 96, 48, 48);
        TextView title = new TextView(this);
        title.setText("An agent asks to continue");
        title.setTextSize(22);
        TextView ask = new TextView(this);
        ask.setText(Approval.text());
        ask.setTextSize(20);
        ask.setPadding(0, 32, 0, 32);
        TextView command = new TextView(this);
        command.setText("It will run:\n" + Approval.detail());
        command.setTextSize(14);
        Button allow = new Button(this);
        allow.setText("Allow once");
        allow.setOnClickListener(v -> answer(true));
        Button deny = new Button(this);
        deny.setText("Deny");
        deny.setOnClickListener(v -> answer(false));
        page.addView(title);
        page.addView(ask);
        page.addView(command);
        page.addView(allow);
        page.addView(deny);
        setContentView(page);
    }

    @Override
    protected void onStart() {
        super.onStart();
        handler.post(watch);
    }

    @Override
    protected void onDestroy() {
        handler.removeCallbacks(watch);
        // Leaving the prompt any other way is a refusal.
        Approval.answer(false, SystemClock.elapsedRealtime());
        super.onDestroy();
    }

    @Override
    public void onBackPressed() {
        answer(false);
    }

    private void answer(boolean allow) {
        Approval.answer(allow, SystemClock.elapsedRealtime());
        finish();
    }
}
