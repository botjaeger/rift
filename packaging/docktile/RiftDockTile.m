// Rift's Dock tile plugin (Contents/PlugIns/RiftDockTile.docktileplugin,
// built by packaging/make-app.sh). The Dock loads it to draw Rift's icon
// while Rift is not running: the `app_icon` the app last applied, which it
// stores as `AppIcon` in its defaults (dev.rift.terminal) and announces
// with the distributed notification dev.rift.terminal.AppIconChanged
// (crates/rift-terminal/src/icon.rs). The default leaves the bundle icon.
#import <AppKit/AppKit.h>

static NSString *const kDomain = @"dev.rift.terminal";
static NSString *const kChanged = @"dev.rift.terminal.AppIconChanged";

@interface RiftDockTilePlugIn : NSObject <NSDockTilePlugIn>
@property (strong) NSDockTile *tile;
@end

@implementation RiftDockTilePlugIn

- (void)show {
    NSDockTile *tile = self.tile;
    if (tile == nil) {
        return;
    }
    CFPreferencesAppSynchronize((__bridge CFStringRef)kDomain);
    NSString *name = CFBridgingRelease(
        CFPreferencesCopyAppValue(CFSTR("AppIcon"), (__bridge CFStringRef)kDomain));
    NSImage *image = nil;
    // names come from the app's own list; anything else draws the default
    if ([name isKindOfClass:[NSString class]] && ![name isEqualToString:@"default"] &&
        [name rangeOfCharacterFromSet:[[NSCharacterSet alphanumericCharacterSet] invertedSet]]
                .location == NSNotFound) {
        NSString *path = [[NSBundle bundleForClass:[self class]] pathForResource:name
                                                                          ofType:@"png"];
        if (path != nil) {
            image = [[NSImage alloc] initWithContentsOfFile:path];
        }
    }
    if (image == nil) {
        tile.contentView = nil;
    } else {
        NSImageView *view = [NSImageView imageViewWithImage:image];
        view.imageScaling = NSImageScaleProportionallyUpOrDown;
        tile.contentView = view;
    }
    [tile display];
}

- (void)setDockTile:(NSDockTile *)dockTile {
    NSDistributedNotificationCenter *center = [NSDistributedNotificationCenter defaultCenter];
    [center removeObserver:self];
    self.tile = dockTile;
    if (dockTile != nil) {
        [center addObserver:self
                   selector:@selector(changed:)
                       name:kChanged
                     object:nil
         suspensionBehavior:NSNotificationSuspensionBehaviorDeliverImmediately];
        [self show];
    }
}

- (void)changed:(NSNotification *)note {
    [self show];
}

@end
