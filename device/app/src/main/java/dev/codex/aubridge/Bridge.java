package dev.codex.aubridge;

import android.app.KeyguardManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ResolveInfo;
import android.net.Credentials;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.PowerManager;
import android.os.SystemClock;
import android.provider.Settings;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Locale;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

final class Bridge implements AutoCloseable {
    static final int MAX_FRAME = 1_048_576;
    private final Context context;
    private final Capture capture;
    private final Vm vm;
    private final byte[] token = new byte[32], nonce = new byte[32];
    private final ThreadPoolExecutor workers =
            new ThreadPoolExecutor(
                    2,
                    2,
                    30,
                    TimeUnit.SECONDS,
                    new ArrayBlockingQueue<>(8),
                    r -> {
                        Thread t = new Thread(r, "au-worker");
                        t.setDaemon(true);
                        return t;
                    },
                    new ThreadPoolExecutor.AbortPolicy());

    /** When the host last sent a request (uptime ms), for the app's status screen; 0 = never. */
    static volatile long lastContact;

    private volatile boolean open = true;
    private final Runnable onBindFailure;
    private LocalServerSocket bootstrap, command;

    Bridge(Context context, Runnable onBindFailure) {
        this.context = context;
        this.onBindFailure = onBindFailure;
        new SecureRandom().nextBytes(token);
        new SecureRandom().nextBytes(nonce);
        capture = new Capture(context);
        vm = new Vm(context, capture);
    }

    void start() {
        new Thread(() -> listen(true), "au-bootstrap").start();
        new Thread(() -> listen(false), "au-command").start();
    }

    private void listen(boolean boot) {
        String name = boot ? "aubridge-bootstrap-v3" : "aubridge-v3";
        LocalServerSocket server;
        try {
            server = Serve.bind(() -> new LocalServerSocket(name), 6, 250);
        } catch (IOException e) {
            if (open) onBindFailure.run();
            return;
        }
        if (boot) bootstrap = server;
        else command = server;
        Serve.loop(
                server::accept,
                socket -> {
                    try {
                        workers.execute(() -> handle(socket, boot));
                    } catch (RuntimeException e) {
                        socket.close();
                    }
                },
                () -> open);
    }

    private void handle(LocalSocket socket, boolean boot) {
        try (LocalSocket s = socket) {
            s.setSoTimeout(35_000);
            Credentials peer = s.getPeerCredentials();
            if (peer == null || peer.getUid() != 2000) return;
            DataInputStream in = new DataInputStream(new BufferedInputStream(s.getInputStream()));
            DataOutputStream out =
                    new DataOutputStream(new BufferedOutputStream(s.getOutputStream()));
            if (boot) {
                JSONArray q = read(in);
                byte[] challenge = q.length() == 3 ? unb64(q.optString(2)) : new byte[0];
                if (q.optLong(0, -1) != 0
                        || !"bootstrap".equals(q.optString(1))
                        || challenge.length < 16
                        || challenge.length > 64) return;
                byte[] proof = HelperKey.respond(HelperKey.key(), challenge);
                write(
                        out,
                        new JSONArray()
                                .put(0)
                                .put(0)
                                .put(b64(token))
                                .put(b64(nonce))
                                .put(b64(proof)));
                return;
            }
            JSONArray hello = read(in);
            byte[] challenge = auth(hello);
            if (challenge == null) {
                write(out, new JSONArray().put(0).put(7));
                return;
            }
            write(
                    out,
                    new JSONArray()
                            .put(0)
                            .put(0)
                            .put(
                                    b64(
                                            HelperKey.respondCommand(
                                                    HelperKey.key(), challenge, b64(token)))));
            long seq = 0;
            while (open) {
                JSONArray q;
                try {
                    q = read(in);
                } catch (EOFException e) {
                    return;
                }
                long got = q.optLong(0, -1);
                if (got != seq + 1) {
                    write(out, new JSONArray().put(got).put(8));
                    return;
                }
                seq = got;
                write(out, dispatch(q));
            }
        } catch (Exception ignored) {
        }
    }

    /** The host's challenge if the hello carries this process's token and nonce, else null. */
    private byte[] auth(JSONArray q) {
        if (q.length() != 5 || q.optLong(0, -1) != 0 || !"hello".equals(q.optString(1)))
            return null;
        byte[] challenge = unb64(q.optString(4));
        boolean ok =
                MessageDigest.isEqual(token, unb64(q.optString(2)))
                        & MessageDigest.isEqual(nonce, unb64(q.optString(3)))
                        & challenge.length >= 16
                        & challenge.length <= 64;
        return ok ? challenge : null;
    }

    private JSONArray dispatch(JSONArray q) {
        long seq = q.optLong(0);
        try {
            String name = q.getString(1);
            lastContact = SystemClock.elapsedRealtime();
            if (BridgeService.stopped(context) && !"env".equals(name) && !"status".equals(name))
                return err(seq, 11);
            Ui ui = Ui.get();
            switch (name) {
                case "status":
                    return new JSONArray()
                            .put(seq)
                            .put(0)
                            .put(ui == null ? 0 : ui.generation())
                            .put(ui == null ? 0 : 7 | TreeWindows.CAPABILITY | Approval.CAPABILITY);
                case "observe":
                    if (ui == null) return err(seq, 10);
                    return ui.observe(seq, q.isNull(2) ? null : q.optString(2), q.optInt(3));
                case "semantic":
                    if (ui == null) return err(seq, 10);
                    return new JSONArray()
                            .put(seq)
                            .put(0)
                            .put(ui.semantic(q.isNull(2) ? "" : q.optString(2)));
                case "tree":
                    if (ui == null) return err(seq, 10);
                    return new JSONArray().put(seq).put(0).put(TreeWindows.dump(ui));
                case "marks":
                    if (ui == null) return err(seq, 10);
                    return new JSONArray().put(seq).put(0).put(ui.marks());
                case "resolve":
                    if (ui == null) return err(seq, 10);
                    org.json.JSONObject target = q.optJSONObject(2);
                    return new JSONArray()
                            .put(seq)
                            .put(0)
                            .put(
                                    ui.resolve(
                                            target == null ? "" : target.optString("label"),
                                            target == null ? "" : target.optString("want"),
                                            target == null ? -1 : target.optInt("handle", -1)));
                case "scroller":
                    if (ui == null) return err(seq, 10);
                    return new JSONArray().put(seq).put(0).put(ui.scroller(q.optString(2)));
                case "capabilities":
                    return new JSONArray().put(seq).put(0).put(Capture.capabilities(context));
                case "env":
                    return new JSONArray().put(seq).put(0).put(env(context));
                case "location":
                    return new JSONArray().put(seq).put(0).put(Location.read(context));
                case "notifications":
                    return new JSONArray().put(seq).put(0).put(Notify.read(context));
                case "approve":
                    {
                        JSONObject ask = q.optJSONObject(2);
                        if (ask == null) return err(seq, 5);
                        if (!Approval.start(
                                ask.optString("text"),
                                ask.optString("detail"),
                                ask.optLong("ms", 120_000),
                                SystemClock.elapsedRealtime())) return err(seq, 6);
                        context.startActivity(
                                new Intent(context, ApproveActivity.class)
                                        .addFlags(
                                                Intent.FLAG_ACTIVITY_NEW_TASK
                                                        | Intent.FLAG_ACTIVITY_CLEAR_TOP));
                        return new JSONArray()
                                .put(seq)
                                .put(0)
                                .put(new JSONObject().put("state", "pending"));
                    }
                case "approval":
                    {
                        String answer = Approval.current(SystemClock.elapsedRealtime());
                        // An answer is read once; the next prompt starts clean.
                        if (!"pending".equals(answer)) Approval.clear();
                        return new JSONArray()
                                .put(seq)
                                .put(0)
                                .put(new JSONObject().put("state", answer));
                    }
                case "apps":
                    return new JSONArray().put(seq).put(0).put(apps(context));
                case "run":
                    return vm.run(seq, q);
                case "artifact":
                    return capture.read(
                            seq,
                            q.optString(2),
                            q.isNull(3) ? null : q.optLong(3),
                            q.isNull(4) ? null : q.optLong(4));
                default:
                    return err(seq, 5);
            }
        } catch (Exception e) {
            return err(seq, e instanceof Limit ? 6 : e instanceof Vm.Permission ? 10 : 9);
        }
    }

    /** What the phone is doing, for the host's preflight and status. */
    static JSONObject env(Context context) throws JSONException {
        PowerManager power = context.getSystemService(PowerManager.class);
        String on =
                Settings.Secure.getString(
                        context.getContentResolver(),
                        Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES);
        on = on == null ? "" : on.toLowerCase(Locale.ROOT);
        return new JSONObject()
                .put("screen_on", power.isInteractive())
                .put("locked", context.getSystemService(KeyguardManager.class).isKeyguardLocked())
                .put("accessibility", Ui.get() != null)
                .put(
                        "battery_exempt",
                        power.isIgnoringBatteryOptimizations(context.getPackageName()))
                .put("talkback", on.contains("talkback"))
                .put("stopped", BridgeService.stopped(context));
    }

    private static final long APPS_TTL_MS = 30_000;
    private static JSONArray appsCache;
    private static long appsCachedAt;

    /**
     * Launcher apps as [package, label]. Loading labels reads each app's resources, which is slow,
     * so the list is cached briefly; an app installed moments ago appears after the TTL.
     */
    private static synchronized JSONArray apps(Context context) {
        long now = android.os.SystemClock.elapsedRealtime();
        if (appsCache != null && now - appsCachedAt < APPS_TTL_MS) return appsCache;
        JSONArray out = new JSONArray();
        Intent intent = new Intent(Intent.ACTION_MAIN);
        intent.addCategory(Intent.CATEGORY_LAUNCHER);
        java.util.HashSet<String> seen = new java.util.HashSet<>();
        android.content.pm.PackageManager pm = context.getPackageManager();
        for (ResolveInfo info : pm.queryIntentActivities(intent, 0)) {
            String pkg = info.activityInfo == null ? "" : info.activityInfo.packageName;
            if (pkg.isEmpty() || seen.contains(pkg)) continue;
            CharSequence loaded = info.loadLabel(pm);
            String label = loaded == null ? pkg : loaded.toString();
            if (label.length() > 128) label = label.substring(0, 128);
            out.put(new JSONArray().put(pkg).put(label));
            seen.add(pkg);
            if (out.length() >= 128) break;
        }
        appsCache = out;
        appsCachedAt = now;
        return out;
    }

    static JSONArray err(long seq, int code) {
        return new JSONArray().put(seq).put(code);
    }

    static JSONArray read(DataInputStream in) throws IOException, JSONException {
        int n = in.readInt();
        if (n <= 0 || n > MAX_FRAME) throw new IOException("frame");
        byte[] b = new byte[n];
        in.readFully(b);
        return new JSONArray(new String(b, StandardCharsets.UTF_8));
    }

    static void write(DataOutputStream out, JSONArray value) throws IOException {
        byte[] b = value.toString().getBytes(StandardCharsets.UTF_8);
        if (b.length > MAX_FRAME) throw new IOException("frame");
        out.writeInt(b.length);
        out.write(b);
        out.flush();
    }

    private static String b64(byte[] b) {
        return Base64.encodeToString(b, Base64.NO_WRAP | Base64.URL_SAFE);
    }

    private static byte[] unb64(String s) {
        try {
            return Base64.decode(s, Base64.NO_WRAP | Base64.URL_SAFE);
        } catch (IllegalArgumentException e) {
            return new byte[0];
        }
    }

    @Override
    public void close() {
        open = false;
        try {
            if (bootstrap != null) bootstrap.close();
        } catch (IOException ignored) {
        }
        try {
            if (command != null) command.close();
        } catch (IOException ignored) {
        }
        workers.shutdownNow();
    }

    static final class Limit extends Exception {
        Limit() {
            super("limit");
        }
    }
}
