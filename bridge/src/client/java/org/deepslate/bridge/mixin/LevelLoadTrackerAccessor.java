package org.deepslate.bridge.mixin;

import net.minecraft.client.multiplayer.LevelLoadTracker;
import net.minecraft.server.level.progress.LevelLoadListener;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

@Mixin(LevelLoadTracker.class)
public interface LevelLoadTrackerAccessor {
    @Accessor("serverStage")
    LevelLoadListener.Stage deepslate$getServerStage();
}
