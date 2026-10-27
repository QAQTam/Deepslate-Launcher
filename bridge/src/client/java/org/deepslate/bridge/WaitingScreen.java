package org.deepslate.bridge;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

/** A temporary launcher-owned screen, shown only while a managed operation is pending. */
final class WaitingScreen extends Screen {
    private Component message = BridgeClient.phaseComponent("ready");

    WaitingScreen() {
        super(Component.literal("深板岩启动器"));
    }

    void setMessageKey(String key) {
        this.message = BridgeClient.phaseComponent(key);
    }

    @Override
    public boolean shouldCloseOnEsc() {
        return false;
    }

    @Override
    public void extractRenderState(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float delta) {
        graphics.fill(0, 0, width, height, 0xff272b2f);
        int panelWidth = Math.min(520, Math.max(240, width - 32));
        int left = (width - panelWidth) / 2;
        int center = height / 2;
        graphics.fill(left + 3, center - 53, left + panelWidth + 3, center + 54, 0x70000000);
        graphics.fill(left, center - 56, left + panelWidth, center + 51, 0xffc6b28d);
        graphics.fill(left + 2, center - 54, left + panelWidth - 2, center + 49, 0xffeaddbc);
        graphics.text(font, "DEEPSLATE / GAME SESSION", left + 18, center - 38, 0xff59605c);
        graphics.text(font, message, left + 18, center - 13, 0xff30363a);
        graphics.text(font, BridgeClient.phaseComponent("session_waiting"), left + 18, center + 7, 0xff77786d);
        int barLeft = left + 18;
        int barWidth = panelWidth - 36;
        graphics.fill(barLeft, center + 32, barLeft + barWidth, center + 38, 0xffb4a78f);
        int segment = Math.max(28, barWidth / 4);
        int travel = Math.max(1, barWidth - segment);
        int moving = (int) ((System.currentTimeMillis() / 12L) % (travel * 2L));
        int offset = moving <= travel ? moving : travel * 2 - moving;
        graphics.fill(barLeft + offset, center + 32, barLeft + offset + segment, center + 38, 0xff4c5c5d);
    }
}
