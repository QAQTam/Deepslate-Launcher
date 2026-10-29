package org.deepslate.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.authlib.services.FriendsService;
import com.mojang.authlib.services.response.FriendData;
import com.mojang.authlib.services.response.FriendDto;
import com.mojang.authlib.services.response.PresenceResponse;
import com.mojang.authlib.services.response.PresenceStatus;
import com.mojang.authlib.services.response.PresenceStatusDto;

import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.function.Consumer;

/** In-memory FriendsService mirror of the launcher's authoritative friend store. */
public final class DeepslateFriendsService implements FriendsService {
    private static final DeepslateFriendsService INSTANCE = new DeepslateFriendsService();
    private static final Set<String> PRESENCE_STATES = Set.of(
            "online", "playing_offline", "playing_realms", "playing_server",
            "playing_hosted_server", "offline");
    private final Store store = new Store();
    private boolean ready;
    private String lastReportedStatus = "";

    private DeepslateFriendsService() { }

    public static DeepslateFriendsService get() {
        return INSTANCE;
    }

    public synchronized String getInviteCode() {
        return store.inviteCode;
    }

    /** Replace the game-side mirror with a validated snapshot sent by the launcher. */
    public synchronized void replaceFromLauncher(JsonObject snapshot) {
        store.inviteCode = string(snapshot, "invite_code", "");
        store.friends = parseEntries(snapshot.getAsJsonArray("friends"));
        store.incoming = parseEntries(snapshot.getAsJsonArray("incoming_requests"));
        store.outgoing = parseEntries(snapshot.getAsJsonArray("outgoing_requests"));
        store.enabled = bool(snapshot, "enabled");
        store.allowRequests = bool(snapshot, "allow_requests");
        store.presence.clear();
        lastReportedStatus = "";
        JsonArray friends = snapshot.getAsJsonArray("friends");
        if (friends != null) {
            for (JsonElement element : friends) {
                if (!element.isJsonObject()) continue;
                JsonObject entry = element.getAsJsonObject();
                try {
                    UUID id = UUID.fromString(string(entry, "profile_id", ""));
                    String presence = string(entry, "presence", "");
                    if (!PRESENCE_STATES.contains(presence)) continue;
                    store.presence.put(id, new PresenceInfo(presence, longValue(entry, "last_seen")));
                } catch (IllegalArgumentException ignored) { }
            }
        }
        ready = true;
    }

    @Override public synchronized ResultCode getFriendData(Consumer<FriendData> consumer) {
        if (!ready) return ResultCode.SERVICE_NOT_AVAILABLE;
        consumer.accept(new FriendData(store.friends, store.incoming, store.outgoing));
        return ResultCode.SUCCESS;
    }

    @Override public ResultCode removeFriend(UUID profileId) {
        return submit("remove", "profile_id", profileId.toString());
    }

    @Override public ResultCode acceptIncomingFriendRequest(UUID profileId) {
        return submit("accept", "profile_id", profileId.toString());
    }

    @Override public ResultCode declineIncomingFriendRequest(UUID profileId) {
        return submit("decline", "profile_id", profileId.toString());
    }

    @Override public ResultCode sendFriendRequest(String inviteCode) {
        String code = normalizeInviteCode(inviteCode);
        if (code == null) return ResultCode.UNKNOWN_PROFILE;
        synchronized (this) {
            if (!ready) return ResultCode.SERVICE_NOT_AVAILABLE;
            if (code.equalsIgnoreCase(store.inviteCode)) return ResultCode.FORBIDDEN;
        }
        return submit("add_request", "invite_code", code);
    }

    @Override public ResultCode sendFriendRequest(UUID profileId) {
        return ResultCode.UNKNOWN_PROFILE;
    }

    @Override public ResultCode revokeOutgoingFriendRequest(UUID profileId) {
        return submit("cancel", "profile_id", profileId.toString());
    }

    @Override public ResultCode updateFriendSettings(boolean enabled, boolean allowRequests) {
        JsonObject action = action("settings");
        action.addProperty("enabled", enabled);
        action.addProperty("allow_requests", allowRequests);
        synchronized (this) {
            if (!ready) return ResultCode.SERVICE_NOT_AVAILABLE;
        }
        return BridgeClient.submitFriendAction(action)
                ? ResultCode.SUCCESS : ResultCode.SERVICE_NOT_AVAILABLE;
    }

    @Override public PresenceResponse presence(String status) {
        // The native PresenceHandler calls this with our own Authlib status.
        // Forward state transitions to the launcher, which broadcasts them over
        // the decentralized friend network.
        String reported = status == null ? "" : status.trim().toUpperCase(Locale.ROOT);
        if (!reported.isEmpty() && reported.length() <= 32 && !reported.equals(lastReportedStatus)
                && BridgeClient.reportSelfPresence(reported)) {
            lastReportedStatus = reported;
        }
        List<PresenceStatusDto> statuses = new ArrayList<>();
        synchronized (this) {
            if (!ready) return PresenceResponse.empty();
            Instant now = Instant.now();
            for (Map.Entry<UUID, PresenceInfo> entry : store.presence.entrySet()) {
                PresenceInfo info = entry.getValue();
                Instant updated = info.lastSeen > 0 ? Instant.ofEpochMilli(info.lastSeen) : now;
                statuses.add(new PresenceStatusDto(
                        entry.getKey(), entry.getKey(), toPresenceStatus(info.state), updated));
            }
        }
        return new PresenceResponse(statuses);
    }

    @Override public Optional<Duration> getFriendsPollInterval() {
        return Optional.of(Duration.ofSeconds(1));
    }

    @Override public Optional<Duration> getPresencePollInterval() {
        return Optional.of(Duration.ofSeconds(10));
    }

    private static PresenceStatus toPresenceStatus(String state) {
        return switch (state) {
            case "playing_offline" -> PresenceStatus.PLAYING_OFFLINE;
            case "playing_realms" -> PresenceStatus.PLAYING_REALMS;
            case "playing_server" -> PresenceStatus.PLAYING_SERVER;
            case "playing_hosted_server" -> PresenceStatus.PLAYING_HOSTED_SERVER;
            case "offline" -> PresenceStatus.OFFLINE;
            default -> PresenceStatus.ONLINE;
        };
    }

    private static long longValue(JsonObject object, String key) {
        if (!object.has(key) || !object.get(key).isJsonPrimitive()) return 0;
        try {
            return object.get(key).getAsLong();
        } catch (RuntimeException ignored) {
            return 0;
        }
    }

    private ResultCode submit(String operation, String key, String value) {
        synchronized (this) {
            if (!ready) return ResultCode.SERVICE_NOT_AVAILABLE;
        }
        JsonObject action = action(operation);
        action.addProperty(key, value);
        return BridgeClient.submitFriendAction(action)
                ? ResultCode.SUCCESS : ResultCode.SERVICE_NOT_AVAILABLE;
    }

    private static JsonObject action(String operation) {
        JsonObject action = new JsonObject();
        action.addProperty("action", operation);
        return action;
    }

    private static List<FriendDto> parseEntries(JsonArray entries) {
        List<FriendDto> result = new ArrayList<>();
        if (entries == null) return result;
        for (JsonElement element : entries) {
            if (!element.isJsonObject()) continue;
            JsonObject entry = element.getAsJsonObject();
            try {
                UUID id = UUID.fromString(string(entry, "profile_id", ""));
                String name = string(entry, "name", "");
                if (!name.isBlank() && name.length() <= 64) result.add(new FriendDto(id, name));
            } catch (IllegalArgumentException ignored) { }
        }
        return result;
    }

    private static String normalizeInviteCode(String input) {
        if (input == null) return null;
        String code = input.trim();
        if (code.length() < 5 || code.length() > 512) return null;
        if (!code.regionMatches(true, 0, "DL2-", 0, 4)) return null;
        int separator = code.indexOf('-', 4);
        if (separator < 0) return null;
        // The public key is hex (case-insensitive) but the base64url endpoint is
        // case-sensitive, so only normalize the key half.
        String key = code.substring(4, separator).toUpperCase(java.util.Locale.ROOT);
        String endpoint = code.substring(separator + 1);
        String normalized = "DL2-" + key + "-" + endpoint;
        if (!normalized.matches("DL2-[0-9A-F]{64}-[A-Za-z0-9_-]{0,400}")) return null;
        return normalized;
    }

    private static String string(JsonObject object, String key, String fallback) {
        return object.has(key) && object.get(key).isJsonPrimitive()
                ? object.get(key).getAsString() : fallback;
    }

    private static boolean bool(JsonObject object, String key) {
        return object.has(key) && object.get(key).isJsonPrimitive()
                && object.getAsJsonPrimitive(key).isBoolean()
                && object.get(key).getAsBoolean();
    }

    private record PresenceInfo(String state, long lastSeen) { }

    private static final class Store {
        String inviteCode = "";
        List<FriendDto> friends = List.of();
        List<FriendDto> incoming = List.of();
        List<FriendDto> outgoing = List.of();
        final Map<UUID, PresenceInfo> presence = new HashMap<>();
        boolean enabled;
        boolean allowRequests;
    }
}
