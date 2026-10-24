package dev.codex.aubench;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.os.Bundle;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.EditText;
import android.widget.ImageButton;
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
        dialog.setOnClickListener(
                view ->
                        new AlertDialog.Builder(this)
                                .setTitle("Benchmark confirmation")
                                .setMessage("Deterministic fixture dialog")
                                .setPositiveButton(
                                        "CONFIRM",
                                        (ignored, which) -> state.setText("Dialog confirmed"))
                                .setNegativeButton("CANCEL", null)
                                .show());
        content.addView(dialog, matchWrap());

        for (int number = 1; number <= 2; number++) {
            Button details = new Button(this);
            details.setText("Details");
            int shown = number;
            details.setOnClickListener(view -> state.setText("Details " + shown + " opened"));
            content.addView(details, matchWrap());
        }

        Button draft = new Button(this);
        draft.setText("Save draft");
        draft.setOnClickListener(view -> state.setText("Saved draft"));
        content.addView(draft, matchWrap());

        Button slow = new Button(this);
        slow.setText("LOAD SLOW CONTENT");
        slow.setOnClickListener(
                view -> {
                    state.setText("Loading");
                    state.postDelayed(() -> state.setText("Slow content ready"), 1500);
                });
        content.addView(slow, matchWrap());

        ImageButton icon = new ImageButton(this);
        icon.setImageResource(android.R.drawable.ic_menu_info_details);
        icon.setOnClickListener(view -> state.setText("Icon tapped"));
        content.addView(icon, matchWrap());

        Button lab = new Button(this);
        lab.setText("OPEN LAB");
        lab.setOnClickListener(view -> startActivity(new Intent(this, LabActivity.class)));
        content.addView(lab, matchWrap());

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
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }
}
