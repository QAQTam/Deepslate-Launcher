package org.deepslate.bridge.mixin;

import org.deepslate.bridge.DeepslateFriendsService;
import org.deepslate.bridge.BridgeClient;
import net.minecraft.client.User;
import net.minecraft.client.gui.components.EditBox;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(targets = "net.minecraft.client.gui.screens.friends.AddFriendWidget")
abstract class AddFriendWidgetMixin {
    @Shadow @Final private EditBox editBox;

    @Inject(method = "<init>", at = @At("RETURN"))
    private void deepslate$allowFullLauncherFriendCode(int width, Runnable refresh, CallbackInfo ci) {
        editBox.setMaxLength(64);
    }

    @Redirect(
            method = "<init>",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/client/User;getName()Ljava/lang/String;"))
    private String deepslate$showInviteCodeInsteadOfProfileName(User user) {
        return BridgeClient.isManagedLaunch()
                ? DeepslateFriendsService.get().getInviteCode()
                : user.getName();
    }

    @Redirect(
            method = "getInvalidInputReason",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/client/User;getName()Ljava/lang/String;"))
    private String deepslate$validateInviteCodeInsteadOfProfileName(User user) {
        return BridgeClient.isManagedLaunch()
                ? DeepslateFriendsService.get().getInviteCode()
                : user.getName();
    }
}
