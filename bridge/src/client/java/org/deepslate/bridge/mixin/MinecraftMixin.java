package org.deepslate.bridge.mixin;

import org.deepslate.bridge.BridgeClient;

import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
abstract class MinecraftMixin {
    @Inject(method = "disconnectFromWorld", at = @At("HEAD"))
    private void deepslate$leaving(Component message, CallbackInfo ci) {
        BridgeClient.beforeDisconnect();
    }
    @Inject(method = "disconnectFromWorld", at = @At("RETURN"))
    private void deepslate$closed(Component message, CallbackInfo ci) {
        BridgeClient.afterDisconnect((Minecraft) (Object) this);
    }
}
