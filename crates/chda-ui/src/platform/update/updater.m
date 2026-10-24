// A separate Sparkle host. It survives the target GUI and never relaunches it.
#import <Cocoa/Cocoa.h>
#import <Sparkle/Sparkle.h>
#include <signal.h>

@interface CHDAUpdater : NSObject <SPUUserDriver, SPUUpdaterDelegate>
@property(nonatomic, strong) SPUUpdater *updater;
@property(nonatomic, copy) NSString *directory;
@property(nonatomic, copy) NSString *version;
@property(nonatomic, copy) void (^installReply)(SPUUserUpdateChoice);
@property(nonatomic, copy) void (^cancelCheck)(void);
@property(nonatomic) uint64_t total;
@property(nonatomic) uint64_t received;
@property(nonatomic) BOOL installing;
@property(nonatomic) pid_t gui;
@property(nonatomic, strong) NSPanel *progressWindow;
@property(nonatomic, strong) NSTextField *progressLabel;
@end
@implementation CHDAUpdater
- (void)emit:(NSDictionary *)event {
    NSData *json = [NSJSONSerialization dataWithJSONObject:event options:0 error:NULL];
    NSFileHandle *file = [NSFileHandle fileHandleForWritingAtPath:[self.directory stringByAppendingPathComponent:@"progress.jsonl"]];
    [file seekToEndOfFile];
    [file writeData:json];
    [file writeData:[@"\n" dataUsingEncoding:NSUTF8StringEncoding]];
    [file closeFile];
}
- (void)phase:(NSString *)phase { [self emit:@{@"phase":phase}]; }
- (void)fail:(NSString *)message {
    [self emit:@{@"phase":@"failed", @"message":message ?: @"Update failed"}];
}
- (void)poll:(NSTimer *)timer {
    NSFileManager *fm = NSFileManager.defaultManager;
    if ([fm fileExistsAtPath:[self.directory stringByAppendingPathComponent:@"cancel"]]) {
        if (self.installReply) { self.installReply(SPUUserUpdateChoiceSkip); self.installReply = nil; }
        else if (self.cancelCheck) { self.cancelCheck(); self.cancelCheck = nil; }
        [self fail:@"Update cancelled. Your sessions are unchanged."];
        [NSApp terminate:nil];
    } else if (self.installReply && [fm fileExistsAtPath:[self.directory stringByAppendingPathComponent:@"install"]]) {
        self.installing = YES;
        self.installReply(SPUUserUpdateChoiceInstall);
        self.installReply = nil;
    } else if (!self.installing && kill(self.gui, 0) != 0) {
        if (self.installReply) self.installReply(SPUUserUpdateChoiceSkip);
        else if (self.cancelCheck) self.cancelCheck();
        [NSApp terminate:nil];
    }
}
- (BOOL)updaterShouldRelaunchApplication:(SPUUpdater *)updater { return NO; }
- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater { return NO; }
- (BOOL)updater:(SPUUpdater *)updater shouldProceedWithUpdate:(SUAppcastItem *)item updateCheck:(SPUUpdateCheck)check error:(NSError **)error {
    BOOL allowed = [item.displayVersionString isEqualToString:self.version] && !item.informationOnlyUpdate;
    if (!allowed && error) *error = [NSError errorWithDomain:@"chda.updater" code:1 userInfo:@{NSLocalizedDescriptionKey:@"The release changed. Check for updates again."}];
    return allowed;
}
- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest *)request reply:(void (^)(SUUpdatePermissionResponse *))reply {
    reply([[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:NO sendSystemProfile:NO]);
}
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancellation {
    self.cancelCheck = cancellation; [self phase:@"checking"];
}
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)item state:(SPUUserUpdateState *)state reply:(void (^)(SPUUserUpdateChoice))reply {
    if (item.informationOnlyUpdate || ![item.displayVersionString isEqualToString:self.version]) {
        reply(SPUUserUpdateChoiceDismiss); [self fail:@"The selected release is no longer available."];
        [NSApp terminate:nil]; return;
    }
    reply(SPUUserUpdateChoiceInstall);
}
- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData *)data {}
- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError *)error {}
- (void)showUpdateNotFoundWithError:(NSError *)error acknowledgement:(void (^)(void))ack {
    [self fail:error.localizedDescription]; ack(); [NSApp terminate:nil];
}
- (void)showUpdaterError:(NSError *)error acknowledgement:(void (^)(void))ack {
    [self fail:error.localizedDescription]; ack(); [NSApp terminate:nil];
}
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancellation {
    self.cancelCheck = cancellation; self.total = 0; self.received = 0;
    [self emit:@{@"phase":@"downloading", @"received":@0, @"total":@0}];
}
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)length { self.total = length; }
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length {
    self.received += length;
    [self emit:@{@"phase":@"downloading", @"received":@(self.received), @"total":@(self.total)}];
}
- (void)showDownloadDidStartExtractingUpdate { self.cancelCheck = nil; [self phase:@"verifying"]; }
- (void)showExtractionReceivedProgress:(double)progress {}
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply {
    self.installReply = reply; [self phase:@"ready"];
}
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)terminated retryTerminatingApplication:(void (^)(void))retry {
    [self phase:@"installing"];
    if (!self.progressWindow) {
        self.progressWindow = [[NSPanel alloc] initWithContentRect:NSMakeRect(0, 0, 360, 110)
            styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
        self.progressWindow.title = @"chda Update";
        self.progressLabel = [NSTextField labelWithString:@"Installing update…"];
        self.progressLabel.frame = NSMakeRect(55, 43, 280, 24);
        NSProgressIndicator *spinner = [[NSProgressIndicator alloc] initWithFrame:NSMakeRect(20, 43, 24, 24)];
        spinner.style = NSProgressIndicatorStyleSpinning;
        [spinner startAnimation:nil];
        [self.progressWindow.contentView addSubview:spinner];
        [self.progressWindow.contentView addSubview:self.progressLabel];
        [self.progressWindow center];
    }
    [self.progressWindow orderFrontRegardless];
}
- (void)showUpdateInstalledAndRelaunched:(BOOL)relaunched acknowledgement:(void (^)(void))ack {
    [self phase:@"installed"]; ack(); [NSApp terminate:nil];
}
- (void)dismissUpdateInstallation {}
- (void)updater:(SPUUpdater *)updater didAbortWithError:(NSError *)error {
    [self fail:error.localizedDescription]; [NSApp terminate:nil];
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 5) return 2;
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        CHDAUpdater *driver = [CHDAUpdater new];
        driver.directory = [NSString stringWithUTF8String:argv[2]];
        driver.version = [NSString stringWithUTF8String:argv[3]];
        NSString *exitFile = [driver.directory stringByAppendingPathComponent:@"helper-exited"];
        [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationWillTerminateNotification
            object:nil queue:nil usingBlock:^(NSNotification *notification) {
                [@"finished\n" writeToFile:exitFile atomically:YES encoding:NSUTF8StringEncoding error:NULL];
            }];
        driver.gui = (pid_t)strtol(argv[4], NULL, 10);
        NSBundle *host = [NSBundle bundleWithPath:[NSString stringWithUTF8String:argv[1]]];
        driver.updater = [[SPUUpdater alloc] initWithHostBundle:host applicationBundle:host userDriver:driver delegate:driver];
        driver.updater.automaticallyChecksForUpdates = NO;
        driver.updater.automaticallyDownloadsUpdates = NO;
        NSError *error = nil;
        if (![driver.updater startUpdater:&error]) { [driver fail:error.localizedDescription]; return 1; }
        [NSTimer scheduledTimerWithTimeInterval:0.1 target:driver selector:@selector(poll:) userInfo:nil repeats:YES];
        [driver.updater checkForUpdates];
        [NSApp run];
    }
    return 0;
}
