package org.deepslate.bridge;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.multiplayer.LevelLoadTracker;
import net.minecraft.server.level.progress.LevelLoadListener;
import org.deepslate.bridge.mixin.LevelLoadTrackerAccessor;
import org.deepslate.bridge.mixin.LevelLoadingScreenAccessor;

/** Small managed-session decoration over vanilla screens; vanilla keeps owning their lifecycle. */
final class BridgeUi {
    private BridgeUi() {}

    static void afterScreenExtract(net.minecraft.client.gui.screens.Screen screen,
                                   GuiGraphicsExtractor graphics,
                                   int mouseX, int mouseY, float delta) {
        if (!(screen instanceof LevelLoadingScreen loadingScreen) || !BridgeClient.isManagedWorldLoading()) return;

        LevelLoadTracker tracker = ((LevelLoadingScreenAccessor) loadingScreen).deepslate$getLoadTracker();
        LevelLoadListener.Stage stage = ((LevelLoadTrackerAccessor) tracker).deepslate$getServerStage();
        String phase = phase(stage);
        int percent = -1;
        if (tracker.hasProgress()) {
            float value = tracker.serverProgress();
            if (Float.isFinite(value)) percent = Math.round(Math.max(0.0f, Math.min(1.0f, value)) * 100.0f);
        }
        BridgeClient.reportProgress(phase, percent);

        int width = graphics.guiWidth();
        int height = graphics.guiHeight();
        int panelWidth = Math.min(440, Math.max(260, width - 28));
        int left = (width - panelWidth) / 2;
        int top = Math.max(12, height - 94);
        int barLeft = left + 14;
        int barWidth = panelWidth - 28;

        graphics.nextStratum();
        graphics.fill(0, 0, width, height, 0x30272b2f);
        graphics.fill(left + 3, top + 4, left + panelWidth + 3, top + 72, 0x70000000);
        graphics.fill(left, top, left + panelWidth, top + 70, 0xffc6b28d);
        graphics.fill(left + 2, top + 2, left + panelWidth - 2, top + 68, 0xffeaddbc);
        graphics.text(loadingScreen.getFont(), "DEEPSLATE / GAME SESSION", left + 14, top + 8, 0xff59605c);
        graphics.text(loadingScreen.getFont(), BridgeClient.phaseComponent(phase),
                left + 14, top + 27, 0xff30363a);
        if (percent >= 0) {
            graphics.text(loadingScreen.getFont(), percent + "%", left + panelWidth - 48, top + 27, 0xff4c5c5d);
        }
        graphics.fill(barLeft, top + 51, barLeft + barWidth, top + 57, 0xffb4a78f);
        if (percent >= 0) {
            graphics.fill(barLeft, top + 51, barLeft + Math.round(barWidth * percent / 100.0f), top + 57, 0xff4c5c5d);
        } else {
            int segment = Math.max(28, barWidth / 4);
            int travel = Math.max(1, barWidth - segment);
            int moving = (int) ((System.currentTimeMillis() / 12L) % (travel * 2L));
            int offset = moving <= travel ? moving : travel * 2 - moving;
            graphics.fill(barLeft + offset, top + 51, barLeft + offset + segment, top + 57, 0xff4c5c5d);
        }
    }

    private static String phase(LevelLoadListener.Stage stage) {
        if (stage == null) return "world_loading";
        return switch (stage) {
            case START_SERVER -> "world_start_server";
            case PREPARE_GLOBAL_SPAWN -> "world_prepare_spawn";
            case LOAD_INITIAL_CHUNKS -> "world_initial_chunks";
            case LOAD_PLAYER_CHUNKS -> "world_player_chunks";
        };
    }
}
