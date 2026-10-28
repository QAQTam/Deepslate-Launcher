package org.deepslate.bridge;

import com.google.gson.JsonObject;
import com.google.gson.JsonArray;
import com.google.gson.JsonParser;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientChunkEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLevelEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientConfigurationConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientLoginConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.fabricmc.fabric.api.resource.v1.ResourceLoader;
import net.fabricmc.fabric.api.resource.v1.reloader.ResourceReloaderKeys;
import net.fabricmc.fabric.api.resource.v1.reloader.SimpleReloadListener;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.ConnectScreen;
import net.minecraft.client.gui.screens.DisconnectedScreen;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.social.PlayerSocialManager;
import net.minecraft.core.registries.Registries;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.resources.PreparableReloadListener;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Difficulty;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import org.deepslate.bridge.mixin.LevelLoadingScreenAccessor;

import java.io.*;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.Collections;
import java.util.UUID;
import java.util.WeakHashMap;
import java.util.concurrent.ConcurrentHashMap;

/** Client-side half of the launcher session. No connection means ordinary Minecraft behavior. */
public final class BridgeClient implements ClientModInitializer {
    private static final int PROTOCOL = 2;
    private static final int MAX_SESSION_BYTES = 65536;
    private static final int MAX_LINE = 2 * 1024 * 1024;
    private static final Identifier RESOURCE_GATE_BEGIN = Identifier.fromNamespaceAndPath("deepslate-bridge", "resource_gate_begin");
    private static final Identifier RESOURCE_GATE_READY = Identifier.fromNamespaceAndPath("deepslate-bridge", "resource_gate_ready");
    private static BridgeClient session;
    private static volatile boolean localizedResourcesReady;
    private static final Set<Screen> screensWithEvents =
            Collections.newSetFromMap(Collections.synchronizedMap(new WeakHashMap<>()));
    private final String id;
    private final String token;
    private final int port;
    private final Set<String> requests = ConcurrentHashMap.newKeySet();
    private volatile PrintWriter output;
    private volatile boolean authenticated;
    private boolean ready;
    private boolean loading;
    private boolean entered;
    private boolean connectingServer;
    private boolean firstChunkReported;
    private boolean friendListenerInstalled;
    private long lastFriendSnapshotNanos;
    private Runnable friendUpdateListener;
    private String operation;
    private volatile String phase = "client_starting";
    private volatile String lastReportedPhase = "";
    private volatile int lastReportedProgress = -1;
    private volatile PendingGameRules pendingGameRules;

    private record PendingGameRules(
            boolean keepInventory,
            boolean mobGriefing,
            boolean daylightCycle,
            boolean weatherCycle,
            boolean hostileMobs,
            boolean pvp) {}

    /** Fabric constructs the entrypoint before the session description is read. */
    public BridgeClient() {
        id = "";
        token = "";
        port = 0;
    }

    private BridgeClient(JsonObject description) {
        id = description.get("session_id").getAsString();
        token = description.get("token").getAsString();
        port = description.get("port").getAsInt();
        if (port < 1 || port > 65535 || id.isBlank() || token.isBlank()) throw new IllegalArgumentException("invalid session");
    }
    @Override public void onInitializeClient() {
        String path = System.getenv("DEEPSLATE_BRIDGE_SESSION");
        if (path == null || path.isBlank()) return;
        try {
            byte[] bytes = Files.readAllBytes(Path.of(path));
            if (bytes.length > MAX_SESSION_BYTES) throw new IOException("session description too large");
            session = new BridgeClient(JsonParser.parseString(new String(bytes, StandardCharsets.UTF_8)).getAsJsonObject());
            registerResourceListeners();
            session.registerFabricEvents();
            Thread thread = new Thread(session::connect, "deepslate-bridge");
            thread.setDaemon(true);
            thread.start();
            ClientTickEvents.END_CLIENT_TICK.register(session::tick);
        } catch (Exception e) {
            System.err.println("Deepslate Bridge session initialization failed: " + e.getMessage());
        }
    }
    private void connect() {
        try (Socket socket = new Socket("127.0.0.1", port)) {
            socket.setTcpNoDelay(true);
            output = new PrintWriter(new OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8), true);
            JsonObject hello = message("Hello", "");
            hello.addProperty("token", token);
            send(hello);
            Reader reader = new InputStreamReader(socket.getInputStream(), StandardCharsets.UTF_8);
            while (true) {
                String line = readLine(reader);
                if (line == null) break;
                JsonObject command = JsonParser.parseString(line).getAsJsonObject();
                if (!command.has("protocol") || command.get("protocol").getAsInt() != PROTOCOL ||
                    !command.has("session_id") || !id.equals(command.get("session_id").getAsString())) continue;
                if (!authenticated) {
                    if (!command.has("type") || !"Welcome".equals(command.get("type").getAsString())) break;
                    authenticated = true;
                    Minecraft.getInstance().execute(() -> report(phase, -1));
                    continue;
                }
                Minecraft.getInstance().execute(() -> handle(command));
            }
        } catch (Exception e) {
            System.err.println("Deepslate Bridge disconnected: " + e.getMessage());
        } finally { authenticated = false; output = null; }
    }
    private static String readLine(Reader reader) throws IOException {
        StringBuilder line = new StringBuilder();
        for (int c; (c = reader.read()) != -1;) {
            if (c == '\n') return line.toString();
            if (line.length() >= MAX_LINE) throw new IOException("message too large");
            line.append((char) c);
        }
        return line.isEmpty() ? null : line.toString();
    }
    private void handle(JsonObject command) {
        String request = command.has("request_id") ? command.get("request_id").getAsString() : "";
        if (request.isBlank() || !requests.add(request)) return;
        String type = command.get("type").getAsString();
        Minecraft mc = Minecraft.getInstance();
        if (type.equals("JoinServer")) {
            String address = command.get("address").getAsString().trim();
            if (!ready || loading || entered || address.isEmpty() || address.length() > 255 ||
                !ServerAddress.isValidAddress(address)) {
                failure(request, "Invalid server address or client state");
                return;
            }
            loading = true;
            connectingServer = true;
            operation = request;
            firstChunkReported = false;
            report("connecting", -1);
            send(message("WorldLoading", request));
            try {
                var server = new ServerData("Deepslate", address, ServerData.Type.OTHER);
                ConnectScreen.startConnecting(mc.gui.screen(), mc, ServerAddress.parseString(address), server, false, null);
            } catch (Exception e) {
                connectingServer = false;
                failure(request, "Server connection failed: " + e.getMessage());
            }
        } else if (type.equals("OpenWorld") || type.equals("CreateWorld")) {
            String map = command.get("map_id").getAsString();
            if (!ready || loading || entered || map.isEmpty() || map.length() > 255 ||
                map.equals(".") || map.equals("..") || map.indexOf('/') >= 0 ||
                map.indexOf('\\') >= 0 || map.indexOf(':') >= 0 || map.indexOf('\0') >= 0) {
                failure(request, "World cannot be opened in the current state or map ID is invalid");
                return;
            }
            if (type.equals("CreateWorld") && Files.exists(mc.gameDirectory.toPath().resolve("saves").resolve(map))) {
                failure(request, "World already exists");
                return;
            }
            loading = true;
            operation = request;
            firstChunkReported = false;
            report(type.equals("CreateWorld") ? "creating_world" : "opening_world", -1);
            if (mc.gui.screen() instanceof WaitingScreen screen) {
                screen.setMessageKey(type.equals("CreateWorld") ? "deepslate-bridge.phase.creating_world" : "deepslate-bridge.phase.opening_world");
            }
            send(message("WorldLoading", request));
            try {
                if (type.equals("OpenWorld")) {
                    mc.createWorldOpenFlows().openWorld(map, () -> {
                        loading = false;
                        failure(request, "World loading was cancelled or failed");
                        if (mc.gui.screen() instanceof WaitingScreen) mc.gui.setScreen(new TitleScreen());
                    });
                } else {
                    String name = command.get("name").getAsString();
                    String mode = command.get("mode").getAsString();
                    String difficultyKey = command.get("difficulty").getAsString();
                    String preset = command.get("preset").getAsString();
                    String seed = command.get("seed").getAsString();
                    PendingGameRules rules = parseGameRules(command.getAsJsonObject("rules"));
                    Difficulty difficulty = switch (difficultyKey) {
                        case "peaceful" -> Difficulty.PEACEFUL;
                        case "easy" -> Difficulty.EASY;
                        case "normal" -> Difficulty.NORMAL;
                        case "hard" -> Difficulty.HARD;
                        default -> null;
                    };
                    if (name.isBlank() || name.length() > 128 ||
                        !(mode.equals("survival") || mode.equals("creative")) ||
                        difficulty == null ||
                        !(preset.equals("normal") || preset.equals("flat"))) {
                        loading = false;
                        failure(request, "Invalid world settings");
                        return;
                    }
                    GameType gameType = GameType.byName(mode);
                    var settings = new LevelSettings(name, gameType,
                            new LevelSettings.DifficultySettings(difficulty, false, false),
                            command.get("cheats").getAsBoolean(), WorldDataConfiguration.DEFAULT);
                    long seedValue = WorldOptions.parseSeed(seed).orElseGet(WorldOptions::randomSeed);
                    var options = new WorldOptions(seedValue, true, false);
                    var key = preset.equals("flat") ? WorldPresets.FLAT : WorldPresets.NORMAL;
                    var fallback = mc.gui.screen();
                    pendingGameRules = rules;
                    mc.createWorldOpenFlows().createFreshLevel(map, settings, options,
                            registries -> registries.lookupOrThrow(Registries.WORLD_PRESET)
                                    .getOrThrow(key).value().createWorldDimensions(), fallback);
                    if (mc.gui.screen() == fallback) {
                        pendingGameRules = null;
                        failure(request, "World creation could not load its data packs");
                    }
                }
            } catch (Exception e) {
                pendingGameRules = null;
                loading = false;
                failure(request, "World loading failed: " + e.getMessage());
            }
        } else if (type.equals("GetSessionState")) {
            JsonObject state = message("SessionState", request);
            state.addProperty("state", entered ? "entered" : loading ? "loading" : ready ? "ready" : "starting");
            send(state);
        } else if (type.equals("QuitClient")) {
            if (entered && mc.level != null) mc.disconnectFromWorld(net.minecraft.network.chat.Component.literal("Saving world"));
            else mc.stop();
        } else failure(request, "Unknown command");
    }

    private static PendingGameRules parseGameRules(JsonObject object) {
        if (object == null) throw new IllegalArgumentException("Missing game rules");
        return new PendingGameRules(
                requiredBoolean(object, "keep_inventory"),
                requiredBoolean(object, "mob_griefing"),
                requiredBoolean(object, "daylight_cycle"),
                requiredBoolean(object, "weather_cycle"),
                requiredBoolean(object, "hostile_mobs"),
                requiredBoolean(object, "pvp"));
    }

    private static boolean requiredBoolean(JsonObject object, String key) {
        if (!object.has(key) || !object.get(key).isJsonPrimitive() ||
                !object.getAsJsonPrimitive(key).isBoolean()) {
            throw new IllegalArgumentException("Invalid game rule: " + key);
        }
        return object.get(key).getAsBoolean();
    }

    private void tick(Minecraft mc) {
        if (authenticated) syncFriendsIfNeeded(mc);
        if (!ready && authenticated && mc.isGameLoadFinished() && mc.gui.screen() instanceof TitleScreen && mc.gui.overlay() == null) {
            mc.gui.setScreen(new WaitingScreen());
            ready = true;
            report("ready", -1);
            send(message("Ready", ""));
        }
        if (!authenticated && mc.gui.screen() instanceof WaitingScreen && !loading) {
            mc.gui.setScreen(new TitleScreen());
        }
        if (loading && connectingServer && (mc.gui.screen() instanceof DisconnectedScreen || mc.gui.screen() instanceof WaitingScreen)) {
            connectingServer = false;
            failure(operation, "Server connection was cancelled or failed");
        }
    }

    /** Read the same Mojang FriendsService state used by Minecraft's native Friends screen. */
    private void syncFriendsIfNeeded(Minecraft mc) {
        PlayerSocialManager social = mc.getPlayerSocialManager();
        if (!friendListenerInstalled) {
            friendUpdateListener = () -> mc.execute(() -> sendFriendSnapshot(mc, social));
            social.addFriendListUpdateListener(friendUpdateListener);
            friendListenerInstalled = true;
            sendFriendSnapshot(mc, social);
            return;
        }
        long now = System.nanoTime();
        if (now - lastFriendSnapshotNanos >= 30_000_000_000L) {
            sendFriendSnapshot(mc, social);
        }
    }

    private void sendFriendSnapshot(Minecraft mc, PlayerSocialManager social) {
        if (!authenticated || mc.getPlayerSocialManager() != social) return;
        JsonObject snapshot = new JsonObject();
        snapshot.addProperty("service_state", social.getFriendListState().name().toLowerCase(Locale.ROOT));
        snapshot.addProperty("enabled", social.isFriendListEnabled());
        snapshot.addProperty("allow_requests", social.isAllowFriendRequests());

        Map<UUID, String> presenceByProfile = new HashMap<>();
        var presence = social.getPresenceHandler().getLatestPresence();
        if (presence != null && presence.presence() != null) {
            for (var status : presence.presence()) {
                if (status.profileId() != null && status.status() != null) {
                    presenceByProfile.put(status.profileId(), status.status().name().toLowerCase(Locale.ROOT));
                }
            }
        }
        snapshot.add("friends", friendEntries(social.getFriends(), presenceByProfile));
        snapshot.add("incoming_requests", friendEntries(social.getIncomingRequests(), Map.of()));
        snapshot.add("outgoing_requests", friendEntries(social.getOutgoingRequests(), Map.of()));
        snapshot.addProperty("updated_at", System.currentTimeMillis());

        JsonObject event = message("FriendSnapshot", "");
        event.add("snapshot", snapshot);
        send(event);
        lastFriendSnapshotNanos = System.nanoTime();
    }

    private static JsonArray friendEntries(
            java.util.List<PlayerSocialManager.PlayerData> players,
            Map<UUID, String> presenceByProfile) {
        JsonArray entries = new JsonArray();
        int count = Math.min(players.size(), 1000);
        for (int index = 0; index < count; index++) {
            PlayerSocialManager.PlayerData player = players.get(index);
            if (player.id() == null || player.name() == null || player.name().isBlank() || player.name().length() > 64) {
                continue;
            }
            JsonObject entry = new JsonObject();
            entry.addProperty("profile_id", player.id().toString());
            entry.addProperty("name", player.name());
            String presence = presenceByProfile.get(player.id());
            if (presence != null) entry.addProperty("presence", presence);
            else entry.add("presence", null);
            entries.add(entry);
        }
        return entries;
    }

    private void registerFabricEvents() {
        ServerLifecycleEvents.SERVER_STARTED.register(this::applyPendingGameRules);
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> report("client_started", -1));
        ClientLevelEvents.AFTER_CLIENT_LEVEL_CHANGE.register((client, level) -> {
            if (loading && level != null) report("world_attached", -1);
        });
        ClientChunkEvents.CHUNK_LOAD.register((level, chunk) -> {
            if (loading && !firstChunkReported) {
                firstChunkReported = true;
                report("first_chunk", -1);
            }
        });

        ClientLoginConnectionEvents.INIT.register((listener, client) -> report("login_started", -1));
        ClientLoginConnectionEvents.QUERY_START.register((listener, client) -> report("authenticating", -1));
        ClientLoginConnectionEvents.DISCONNECT.register((listener, client) -> {
            if (loading && connectingServer) failure(operation, "Server login was rejected or disconnected");
        });
        ClientConfigurationConnectionEvents.START.register((listener, client) -> report("server_configuring", -1));
        ClientConfigurationConnectionEvents.COMPLETE.register((listener, client) -> report("server_configured", -1));
        ClientConfigurationConnectionEvents.DISCONNECT.register((listener, client) -> {
            if (loading && connectingServer) failure(operation, "Server configuration was interrupted");
        });
        ClientPlayConnectionEvents.JOIN.register((listener, sender, client) -> report("server_joined", -1));
        ClientPlayConnectionEvents.DISCONNECT.register((listener, client) -> {
            if (loading && connectingServer) failure(operation, "Server disconnected before the world loaded");
        });

        ScreenEvents.AFTER_INIT.register((client, screen, width, height) -> {
            if (screensWithEvents.add(screen)) {
                ScreenEvents.afterExtract(screen).register(BridgeUi::afterScreenExtract);
                if (screen instanceof LevelLoadingScreen) {
                    ScreenEvents.remove(screen).register(BridgeClient::levelLoadingScreenRemoved);
                }
            }
        });
    }

    private void applyPendingGameRules(MinecraftServer server) {
        PendingGameRules rules = pendingGameRules;
        if (rules == null) return;
        for (ServerLevel level : server.getAllLevels()) {
            GameRules gameRules = level.getGameRules();
            gameRules.set(GameRules.KEEP_INVENTORY, rules.keepInventory(), server);
            gameRules.set(GameRules.MOB_GRIEFING, rules.mobGriefing(), server);
            gameRules.set(GameRules.ADVANCE_TIME, rules.daylightCycle(), server);
            gameRules.set(GameRules.ADVANCE_WEATHER, rules.weatherCycle(), server);
            gameRules.set(GameRules.SPAWN_MONSTERS, rules.hostileMobs(), server);
            gameRules.set(GameRules.PVP, rules.pvp(), server);
        }
        pendingGameRules = null;
        report("rules_applied", -1);
    }

    private static void registerResourceListeners() {
        ResourceLoader loader = ResourceLoader.get(PackType.CLIENT_RESOURCES);
        loader.registerReloadListener(RESOURCE_GATE_BEGIN, marker(() -> localizedResourcesReady = false));
        loader.registerReloadListener(RESOURCE_GATE_READY, marker(() -> {
            localizedResourcesReady = true;
            if (session != null && session.loading) session.report("localized_resources_ready", -1);
        }));
        loader.addListenerOrdering(RESOURCE_GATE_BEGIN, ResourceReloaderKeys.Client.FONTS);
        loader.addListenerOrdering(RESOURCE_GATE_BEGIN, ResourceReloaderKeys.Client.LANGUAGES);
        loader.addListenerOrdering(ResourceReloaderKeys.Client.FONTS, RESOURCE_GATE_READY);
        loader.addListenerOrdering(ResourceReloaderKeys.Client.LANGUAGES, RESOURCE_GATE_READY);
    }

    private static PreparableReloadListener marker(Runnable apply) {
        return new SimpleReloadListener<Boolean>() {
            @Override
            protected Boolean prepare(SharedState sharedState) {
                return Boolean.TRUE;
            }

            @Override
            protected void apply(Boolean prepared, SharedState sharedState) {
                apply.run();
            }
        };
    }

    private static void levelLoadingScreenRemoved(net.minecraft.client.gui.screens.Screen screen) {
        if (!(screen instanceof LevelLoadingScreen levelScreen) || session == null) return;
        var tracker = ((LevelLoadingScreenAccessor) levelScreen).deepslate$getLoadTracker();
        if (!session.loading || !tracker.isLevelReady()) return;
        session.loading = false;
        session.connectingServer = false;
        session.entered = true;
        session.report("world_ready", 100);
        session.send(session.message("WorldEntered", session.operation));
    }
    public static void beforeDisconnect() {
        if (session != null && session.entered) session.send(session.message("WorldLeaving", session.operation));
    }
    public static boolean isManagedStartup() {
        return session != null && !session.ready && !session.entered;
    }
    public static boolean isManagedSession() {
        return session != null;
    }
    public static boolean isManagedWorldLoading() {
        return session != null && session.loading;
    }
    public static boolean localizedResourcesReady() {
        return localizedResourcesReady;
    }
    public static Component loadingTitle() {
        return localizedResourcesReady
                ? Component.translatable("deepslate-bridge.loading.resources")
                : Component.literal("LOADING GAME RESOURCES");
    }
    public static Component loadingFinalizingTitle() {
        return localizedResourcesReady
                ? Component.translatable("deepslate-bridge.loading.finalizing")
                : Component.literal("FINALIZING RESOURCES");
    }
    public static Component loadingSubtitle() {
        return localizedResourcesReady
                ? Component.translatable("deepslate-bridge.loading.subtitle")
                : Component.literal("PREPARING GAME SESSION");
    }
    public static Component phaseComponent(String key) {
        return localizedResourcesReady
                ? Component.translatable("deepslate-bridge.phase." + key)
                : Component.literal(key.replace('_', ' ').toUpperCase(java.util.Locale.ROOT));
    }
    public static void reportProgress(String stage, int progress) {
        if (session != null) session.report(stage, progress);
    }
    private synchronized void report(String nextPhase, int progress) {
        if (nextPhase == null || nextPhase.length() > 64) return;
        this.phase = nextPhase;
        int bounded = progress < 0 ? -1 : Math.max(0, Math.min(100, progress));
        if (nextPhase.equals(lastReportedPhase) &&
                (bounded < 0 || lastReportedProgress >= 0 && bounded - lastReportedProgress < 5 && bounded != 100)) return;
        lastReportedPhase = nextPhase;
        lastReportedProgress = bounded;
        JsonObject event = message("Status", operation == null ? "" : operation);
        event.addProperty("phase", nextPhase);
        if (bounded >= 0) event.addProperty("progress", bounded);
        send(event);
    }
    public static void reportResourceProgress(float progress, boolean done) {
        if (session == null) return;
        if (!done && progress >= 1.0f) {
            session.report("resource_finalizing", 100);
        } else {
            int percent = Float.isFinite(progress) ? Math.round(Math.max(0.0f, Math.min(1.0f, progress)) * 100.0f) : -1;
            session.report("resource_loading", percent);
        }
    }
    public static void afterDisconnect(Minecraft mc) {
        if (session != null && session.entered) {
            session.entered = false;
            session.send(session.message("WorldClosed", session.operation));
            mc.stop();
        }
    }
    private void failure(String request, String reason) {
        loading = false;
        report("operation_failed", -1);
        JsonObject event = message("OperationFailed", request);
        event.addProperty("message", reason);
        send(event);
        Minecraft mc = Minecraft.getInstance();
        if (mc.gui.screen() instanceof WaitingScreen) mc.gui.setScreen(new TitleScreen());
    }
    private JsonObject message(String type, String request) {
        JsonObject result = new JsonObject();
        result.addProperty("protocol", PROTOCOL);
        result.addProperty("session_id", id);
        result.addProperty("request_id", request);
        result.addProperty("type", type);
        return result;
    }
    private synchronized void send(JsonObject value) {
        PrintWriter writer = output;
        if (writer != null) {
            String line = value.toString();
            if (line.getBytes(StandardCharsets.UTF_8).length + 1 > MAX_LINE) {
                System.err.println("Deepslate Bridge message too large; dropped " + value.get("type"));
                return;
            }
            writer.println(line);
        }
    }
}
