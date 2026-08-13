package dev.codex.aubridge;

import android.annotation.SuppressLint;
import android.content.Context;
import android.content.SharedPreferences;
import android.location.Location;
import android.location.LocationManager;
import android.location.provider.ProviderProperties;
import android.os.SystemClock;

import org.json.JSONArray;
import org.json.JSONObject;

final class LocationControl {
    // Never replace Android's built-in gps/network providers. Android may
    // replace an existing provider when addTestProvider receives the same
    // name, so AU owns isolated names and can remove only what it created.
    private static final String[] PROVIDERS = {"au_gps", "au_network"};
    private static final String PREFS = "au_location";

    private LocationControl() {
    }

    static JSONObject handle(Context context, String operation, JSONObject args) throws Exception {
        LocationManager manager = context.getSystemService(LocationManager.class);
        if (operation.equals("location.set")) {
            double latitude = args.optDouble("latitude", Double.NaN);
            double longitude = args.optDouble("longitude", Double.NaN);
            if (Double.isNaN(latitude) || Double.isNaN(longitude)
                    || latitude < -90.0 || latitude > 90.0
                    || longitude < -180.0 || longitude > 180.0) {
                throw new BridgeServer.BridgeError("E_ARGS", "location.set requires bounded latitude and longitude");
            }
            SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            for (String provider : PROVIDERS) {
                ensureProvider(manager, prefs, provider);
                Location location = new Location(provider);
                location.setLatitude(latitude);
                location.setLongitude(longitude);
                location.setAccuracy(5f);
                location.setTime(System.currentTimeMillis());
                location.setElapsedRealtimeNanos(SystemClock.elapsedRealtimeNanos());
                manager.setTestProviderLocation(provider, location);
            }
            prefs.edit()
                    .putFloat("latitude", (float) latitude)
                    .putFloat("longitude", (float) longitude)
                    .putLong("updated_at", System.currentTimeMillis())
                    .apply();
            return state(manager, prefs).put("persistent", true);
        }
        if (operation.equals("location.clear")) {
            SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            for (String provider : PROVIDERS) {
                if (!prefs.getBoolean("owned_" + provider, false)) {
                    continue;
                }
                try {
                    manager.removeTestProvider(provider);
                } catch (Exception error) {
                    throw new BridgeServer.BridgeError("E_LOCATION", "failed to remove AU-owned provider " + provider + ": " + error.getMessage());
                }
            }
            prefs.edit().clear().apply();
            return new JSONObject().put("cleared", true).put("owned_providers_removed", true);
        }
        if (operation.equals("location.status") || operation.equals("location.get")) {
            return state(manager, context.getSharedPreferences(PREFS, Context.MODE_PRIVATE));
        }
        throw new BridgeServer.BridgeError("E_ARGS", "unknown location operation " + operation);
    }

    @SuppressLint("InlinedApi") // ProviderProperties integer constants are compile-time inlined on API 26-30.
    private static void ensureProvider(LocationManager manager, SharedPreferences prefs, String provider) throws Exception {
        if (!prefs.getBoolean("owned_" + provider, false)) {
            try {
                manager.addTestProvider(
                        provider,
                        false,
                        false,
                        false,
                        false,
                        true,
                        true,
                        true,
                        ProviderProperties.POWER_USAGE_LOW,
                        ProviderProperties.ACCURACY_FINE);
                prefs.edit().putBoolean("owned_" + provider, true).apply();
            } catch (IllegalArgumentException existingProvider) {
                throw new BridgeServer.BridgeError("E_LOCATION", "cannot claim existing provider " + provider);
            } catch (SecurityException security) {
                throw new BridgeServer.BridgeError("E_LOCATION", "mock-location permission is not enabled for AU Bridge");
            }
        }
        try {
            manager.setTestProviderEnabled(provider, true);
        } catch (SecurityException security) {
            throw new BridgeServer.BridgeError("E_LOCATION", "mock-location permission is not enabled for AU Bridge");
        }
    }

    private static JSONObject state(LocationManager manager, SharedPreferences prefs) throws Exception {
        JSONArray owned = new JSONArray();
        for (String provider : PROVIDERS) {
            if (prefs.getBoolean("owned_" + provider, false)) {
                owned.put(provider);
            }
        }
        JSONObject result = new JSONObject();
        result.put("gps_enabled", manager.isProviderEnabled(LocationManager.GPS_PROVIDER));
        result.put("owned_providers", owned);
        result.put("updated_at", prefs.getLong("updated_at", 0L));
        if (prefs.contains("latitude") && prefs.contains("longitude")) {
            result.put("latitude", (double) prefs.getFloat("latitude", 0f));
            result.put("longitude", (double) prefs.getFloat("longitude", 0f));
        }
        return result;
    }
}
