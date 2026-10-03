---
title: "Closed service channels repeatedly wake the editor event loop"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 0efc559
---

## Resolution

Commit `0efc559` (`fix(runtime): stop polling service channels after closure`).

Standalone and Unix persistent host loops now gate required service receivers after their first closure notification. Optional Git and workspace receivers are retired when they return None. This drains queued work before observing closure, then lets the loop wait for useful input instead of repeatedly selecting an immediately ready closed channel. Native Windows host guards already handled closure; its shared optional-receiver helper call follows the new generic name.

Coverage: ended_services_deliver_queued_work_and_then_stop_waking_the_loop in src/main.rs checks queued delivery, one closure, readiness of other work, idle waiting, and optional receiver retirement. It failed before the fix; all 51 runyte binary tests pass.

## Report

Several standalone and persistent-host event-loop branches keep polling a
service receiver after it returns `None`. The loop records some service exits
only once, but this suppresses duplicate logging rather than disabling the
ready branch. A closed Tokio receiver remains immediately ready, so a failed
background worker can cause continuous frame preparation and CPU use while the
editor otherwise keeps running.

The affected receive sites include language servers, workspace search, file
and Git monitors, file-picker events, and optional Git/workspace services.
Syntax already disables its branch after closure; plugin and native catalog
receivers use their own optional lifecycle. Check both Unix host and standalone
loops and the native Windows host for the same failure pattern.

Disconnect a service producer in a controlled fixture. Its termination should
be handled once, remaining input/services should continue, and subsequent
iterations should await useful work rather than repeatedly selecting closure.
Do not remove legitimate live-event wakeups or change normal frame pacing.

The native Windows host already retires its optional receivers and guards its
non-optional service receivers. The standalone loop is shared by platforms and
needs the same closure handling as the Unix persistent-host loop. Terminal
output reception does not return a closed-channel sentinel and is unaffected.
