package org.deepslate.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.authlib.services.FriendsService;
import com.mojang.authlib.services.response.FriendData;
import com.mojang.authlib.services.response.FriendDto;
import com.mojang.authlib.services.response.PresenceResponse;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.UUID;
import java.util.function.Consumer;

/** In-memory FriendsService mirror of the launcher's authoritative friend store. */
public final class DeepslateFriendsService implements FriendsService {
    private static final DeepslateFriendsService INSTANCE = new DeepslateFriendsService();
    private final Store store = new Store();
    private boolean ready;

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
        return PresenceResponse.empty();
    }

    @Override public Optional<Duration> getFriendsPollInterval() {
        return Optional.of(Duration.ofSeconds(1));
    }

    @Override public Optional<Duration> getPresencePollInterval() {
        return Optional.empty();
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
        String code = input.trim().toUpperCase(java.util.Locale.ROOT);
        if (!code.matches("DL1-[0-9A-F]{32}")) return null;
        return code;
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

    private static final class Store {
        String inviteCode = "";
        List<FriendDto> friends = List.of();
        List<FriendDto> incoming = List.of();
        List<FriendDto> outgoing = List.of();
        boolean enabled;
        boolean allowRequests;
    }
}
