#import <AppKit/AppKit.h>
#include <stdbool.h>

// Use live AppKit state, avoiding minifb's inverted/stale notification flag.
// A main window may remain marked main while another application is active;
// both conditions are necessary to stop forwarding input after switching apps.
bool waytransfer_window_focused(void *raw_window) {
    NSWindow *window = (NSWindow *)raw_window;
    return [NSApp isActive] && [window isKeyWindow];
}

void waytransfer_window_parent(void *raw_child, void *raw_parent) {
    NSWindow *child = (NSWindow *)raw_child;
    NSWindow *parent = (NSWindow *)raw_parent;
    NSWindow *previous = [child parentWindow];
    if (previous == parent) return;
    if (previous) [previous removeChildWindow:child];
    if (parent) [parent addChildWindow:child ordered:NSWindowAbove];
}
