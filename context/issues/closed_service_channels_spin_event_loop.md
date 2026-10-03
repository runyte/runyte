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
