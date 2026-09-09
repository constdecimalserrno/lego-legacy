# Guest threads are cooperative on one host thread

Alpha Team creates a second guest thread; Rock Raiders creates none. Guest threads are scheduled
cooperatively by the machine: one host thread runs them all, switching at a declared quantum of
retired instructions and at blocking waits. A schedule that depends on the host's threads would
make every run of a threaded title non-reproducible, and the input script would stop being a
replay.
