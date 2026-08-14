package dev.codex.aubench;

import android.app.Activity;
import android.app.AlertDialog;
import android.os.Bundle;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

/** Permission-free, deterministic UI used only by the live benchmark suite. */
public final class MainActivity extends Activity {
    private TextView state;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        int padding = Math.round(20 * getResources().getDisplayMetrics().density);
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(padding, padding, padding, padding);

        TextView title = new TextView(this);
        title.setText("AU agentic benchmark fixture");
        title.setTextSize(24f);
        content.addView(title, matchWrap());

        EditText name = new EditText(this);
        name.setHint("Fixture text");
        name.setContentDescription("Benchmark text input");
        content.addView(name, matchWrap());

        Button submit = new Button(this);
        submit.setText("SUBMIT FORM");
        submit.setContentDescription("Benchmark submit");
        submit.setOnClickListener(view -> state.setText("Submitted: " + name.getText()));
        content.addView(submit, matchWrap());

        Switch toggle = new Switch(this);
        toggle.setText("Benchmark toggle");
        toggle.setContentDescription("Benchmark toggle");
        toggle.setOnCheckedChangeListener((button, checked) -> state.setText("Toggle=" + checked));
        content.addView(toggle, matchWrap());

        Button dialog = new Button(this);
        dialog.setText("OPEN BENCHMARK DIALOG");
        dialog.setContentDescription("Benchmark dialog");
        dialog.setOnClickListener(view -> new AlertDialog.Builder(this)
                .setTitle("Benchmark confirmation")
                .setMessage("Deterministic fixture dialog")
                .setPositiveButton("CONFIRM", (ignored, which) -> state.setText("Dialog confirmed"))
                .setNegativeButton("CANCEL", null)
                .show());
        content.addView(dialog, matchWrap());

        state = new TextView(this);
        state.setText("Ready");
        state.setTextSize(18f);
        state.setContentDescription("Benchmark state");
        content.addView(state, matchWrap());

        for (int index = 1; index <= 60; index++) {
            TextView row = new TextView(this);
            row.setText("Fixture row " + index);
            row.setTextSize(17f);
            row.setPadding(0, padding / 2, 0, padding / 2);
            content.addView(row, matchWrap());
        }

        ScrollView scroll = new ScrollView(this);
        scroll.setContentDescription("Benchmark scroll surface");
        scroll.addView(content);
        setContentView(scroll);
    }

    private static LinearLayout.LayoutParams matchWrap() {
        return new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT);
    }
}
