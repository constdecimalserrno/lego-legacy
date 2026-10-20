# Guest threads are cooperative on one host thread

Alpha Team creates three guest threads, two of them A3D's; Rock Raiders and Hot Wheels create
none. Guest threads are scheduled cooperatively by the machine: one host thread runs them all,
switching at a declared quantum of retired instructions and at blocking waits. A schedule that
depends on the host's threads would make every run of a threaded title non-reproducible, and the
input script would stop being a replay. So would one that depends on who is watching: a run traced
to every instruction, or stepped in the debugger, keeps the schedule a route's run keeps and
switches at the same instructions (#131).

## Waiting is parking, not asking again

A thread that cannot go on is put down, not left runnable. It says what it is waiting for, one thing
or several, and, when it asked for a timeout, the declared millisecond it may be woken at; whoever
signals any of them — `SetEvent`, `ReleaseMutex`, `ReleaseSemaphore`, `LeaveCriticalSection`, or a
thread ending — picks the waiters back up, and a waiter that finds it cannot take what it waited
for, or not all of it, goes back down until its hour (#130).
Nothing else can run it in between, so when every thread is down the machine moves the declared
clock to the earliest hour any of them asked for rather than spinning between them. A thread that
sleeps for no time at all, `Sleep(0)`, gives the rest of its turn to the next thread in the
machine's order that can run, as Windows gives it to the next ready thread, and carries on at once
when there is none (#115).

This has to be so because of what the timebase is (ADR-0003): declared time comes from retired
instructions, and a thread that answers a wait by asking again retires none. Alpha Team's A3D
threads wait ten and twenty milliseconds at a time while the title's own thread takes a critical
section one of them holds; leaving that thread runnable meant every thread alive, no instruction
retired, the clock stopped, and so nobody's hour ever came. A wait must also remember: an event
keeps whether it has been signalled and whether it is manual, and a mutex keeps who holds it and
how many times over, or there is nothing for a waiter to be woken by (#60).

A wait that no other thread could ever answer is not a wait: a title running on one thread waits on
an event a device would have signalled, and on this machine the device is the machine, so the wait
is over as soon as it is asked for — which is what it has always done here, and what Hot Wheels'
race is paced by. And when every thread left is waiting for another with no hour at all, the run
stops and says so, rather than looking busy for ever.
