package org.deepslate.bridge.mixin;

import org.deepslate.bridge.BridgeClient;
import org.deepslate.bridge.DeepslateFriendsService;

import com.mojang.authlib.services.FriendsService;
import com.mojang.authlib.services.MinecraftServicesDiscoveryService;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
abstract class MinecraftMixin {
    @Redirect(
            method = "<init>",
            at = @At(value = "INVOKE", target = "Lcom/mojang/authlib/services/MinecraftServicesDiscoveryService;createFriendsService(Ljava/lang/String;)Lcom/mojang/authlib/services/FriendsService;"))
    private FriendsService deepslate$replaceFriendsService(
            MinecraftServicesDiscoveryService discoveryService,
            String accessToken) {
        return BridgeClient.isManagedLaunch()
                ? DeepslateFriendsService.get()
                : discoveryService.createFriendsService(accessToken);
    }

    @Inject(method = "disconnectFromWorld", at = @At("HEAD"))
    private void deepslate$leaving(Component message, CallbackInfo ci) {
        BridgeClient.beforeDisconnect();
    }
    @Inject(method = "disconnectFromWorld", at = @At("RETURN"))
    private void deepslate$closed(Component message, CallbackInfo ci) {
        BridgeClient.afterDisconnect((Minecraft) (Object) this);
    }
}
