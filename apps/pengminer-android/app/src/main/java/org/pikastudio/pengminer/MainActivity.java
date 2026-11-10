package org.pikastudio.pengminer;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.os.*;
import android.graphics.Color;
import android.text.InputType;
import android.view.View;
import android.widget.*;

public final class MainActivity extends Activity {
    static volatile String miningSummary = "";
    private final Handler handler = new Handler(Looper.getMainLooper());
    private EditText server;
    private EditText wallet;
    private EditText password;
    private EditText network;
    private Spinner intensity;
    private TextView status;
    private Button start;
    private Button stop;
    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            status.setText(MiningService.running ? miningSummary : MiningService.status);
            boolean idle = !MiningService.running;
            start.setEnabled(idle);
            stop.setEnabled(!idle);
            for (View field : new View[] {server, wallet, password, network, intensity}) field.setEnabled(idle);
            handler.postDelayed(this, 500);
        }
    };

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int padding = (int) (24 * getResources().getDisplayMetrics().density);
        layout.setPadding(padding, padding, padding, padding);
        layout.setBackgroundColor(Color.rgb(16, 25, 37));
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.addView(layout);
        setContentView(scroll);
        TextView title = text("PENG / MINER", 28);
        layout.addView(title);
        layout.addView(text("Real PCA-10 CPU mining • Android", 16));
        layout.addView(text("Mining uses battery and generates heat. Start only on a device you control. One CPU worker; stops at severe thermal status. Rewards are confirmed by your server.", 14));
        server = field(layout, "HTTPS server", "https://pengchain.pikastudio.org", false);
        wallet = field(layout, "Existing wallet ID", "", false);
        password = field(layout, "Password (held in memory only)", "", true);
        network = field(layout, "Network ID", "pengchain-v1", false);
        layout.addView(text("CPU intensity", 14));
        intensity = new Spinner(this);
        intensity.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item,
                new String[] {"25%", "50%", "100%"}));
        layout.addView(intensity);
        start = new Button(this);
        start.setText(R.string.start_mining);
        layout.addView(start);
        stop = new Button(this);
        stop.setText(R.string.stop_mining);
        layout.addView(stop);
        layout.addView(text("Sessions are not saved. Mining continues with a notification when this screen closes. Android may stop the service; it never restarts itself.", 14));
        status = text(MiningService.status, 18);
        status.setTextIsSelectable(true);
        layout.addView(status);
        start.setOnClickListener(view -> {
            String secret = password.getText().toString();
            if (secret.isEmpty() || wallet.getText().toString().trim().isEmpty()
                    || network.getText().toString().trim().isEmpty()) {
                Toast.makeText(this, "Enter wallet, password, and network ID", Toast.LENGTH_LONG).show();
                return;
            }
            try {
                new PengApi(server.getText().toString().trim(), wallet.getText().toString().trim());
                Intent intent = new Intent(this, MiningService.class)
                        .putExtra("server", server.getText().toString().trim())
                        .putExtra("wallet", wallet.getText().toString().trim())
                        .putExtra("password", secret)
                        .putExtra("network", network.getText().toString().trim())
                        .putExtra("intensity", new int[] {25, 50, 100}[intensity.getSelectedItemPosition()]);
                password.setText("");
                miningSummary = "Starting…";
                startForegroundService(intent);
            } catch (Exception error) {
                Toast.makeText(this, error.getMessage(), Toast.LENGTH_LONG).show();
            }
        });
        stop.setOnClickListener(view -> startService(new Intent(this, MiningService.class).setAction(MiningService.STOP)));
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
    }

    private TextView text(String value, int size) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(Color.rgb(224, 237, 244));
        view.setPadding(0, 12, 0, 12);
        return view;
    }

    private EditText field(LinearLayout layout, String label, String value, boolean secret) {
        layout.addView(text(label, 14));
        EditText view = new EditText(this);
        view.setSingleLine(true);
        view.setTextColor(Color.WHITE);
        view.setInputType(secret ? InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD
                : InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        view.setText(value);
        view.setSaveEnabled(!secret);
        if (secret) view.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
        layout.addView(view);
        return view;
    }

    @Override public void onResume() { super.onResume(); handler.post(refresh); }
    @Override public void onPause() { handler.removeCallbacks(refresh); super.onPause(); }
}
