package org.deepslate.bridge.mixin;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.LoadingOverlay;
import net.minecraft.server.packs.resources.ReloadInstance;
import org.deepslate.bridge.BridgeClient;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Draws over the initial overlay without replacing its progress, fade, tick or callback logic. */
@Mixin(LoadingOverlay.class)
abstract class LoadingOverlayMixin {
    @Shadow @Final private Minecraft minecraft;
    @Shadow @Final private ReloadInstance reload;
    @Shadow @Final private boolean fadeIn;

    @Inject(method = "extractRenderState", at = @At("TAIL"))
    private void deepslate$drawLoading(GuiGraphicsExtractor graphics, int mouseX, int mouseY,
                                      float partialTick, CallbackInfo ci) {
        if (!BridgeClient.isManagedStartup()) return;
        float progress = reload.getActualProgress();
        if (!Float.isFinite(progress)) progress = 0;
        progress = Math.max(0, Math.min(1, progress));
        boolean finalizing = progress >= 1.0f && !reload.isDone();
        BridgeClient.reportResourceProgress(progress, reload.isDone());
        if (fadeIn || minecraft.font == null) return;

        int width = graphics.guiWidth();
        int height = graphics.guiHeight();
        int center = height / 2;
        int panelWidth = Math.min(640, Math.max(280, width - 32));
        int left = (width - panelWidth) / 2;
        graphics.nextStratum();
        graphics.fill(0, 0, width, height, 0xff272b2f);
        graphics.fill(left + 4, center - 62, left + panelWidth + 4, center + 63, 0x70000000);
        graphics.fill(left, center - 66, left + panelWidth, center + 59, 0xffc6b28d);
        graphics.fill(left + 2, center - 64, left + panelWidth - 2, center + 57, 0xffeaddbc);
        graphics.centeredText(minecraft.font, "DEEPSLATE", width / 2, center - 45, 0xff30363a);
        graphics.centeredText(minecraft.font,
                finalizing ? BridgeClient.loadingFinalizingTitle() : BridgeClient.loadingTitle(),
                width / 2, center - 19, 0xff30363a);
        graphics.centeredText(minecraft.font, BridgeClient.loadingSubtitle(), width / 2, center + 4, 0xff77786d);
        int barLeft = left + 20;
        int barWidth = panelWidth - 40;
        graphics.fill(barLeft, center + 28, barLeft + barWidth, center + 35, 0xffb4a78f);
        graphics.fill(barLeft, center + 28, barLeft + (int) (barWidth * progress), center + 35, 0xff4c5c5d);
        String percent = Math.round(progress * 100) + "%";
        graphics.centeredText(minecraft.font, percent, width / 2, center + 40, 0xff59605c);
    }
}
