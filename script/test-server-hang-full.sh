#!/bin/bash

# Timeout threshold (seconds)
TIMEOUT=70
COUNTER=1

# Cleanup function to handle process termination
#
cleanup() {
    local pid=$1
    # First try SIGTERM for graceful shutdown
    kill -15 "$pid" 2>/dev/null
    
    # Wait up to 5 seconds for process to terminate
    for _ in {1..5}; do
        if ! kill -0 "$pid" 2>/dev/null; then
            return
        fi
        sleep 1
    done
    
    # If process still running, then use SIGKILL as last resort
    if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null
    fi
}

# Trap script termination
trap 'exit_handler' SIGINT SIGTERM

# Handler for script termination
exit_handler() {
    echo "Received termination signal. Cleaning up..."
    if [ ! -z "$CMD_PID" ]; then
        cleanup "$CMD_PID"
    fi
    exit 0
}

while true; do
    # Run cargo test and redirect output properly
    cargo test -- --nocapture > full-output.txt 2>&1 & 
    CMD_PID=$!
    
    # Monitor the command execution
    SECONDS=0
    while kill -0 "$CMD_PID" 2>/dev/null; do
        sleep 1
        if [[ $SECONDS -ge $TIMEOUT ]]; then
            echo "Command hung for $TIMEOUT seconds. Terminating..."
            echo "Failed during iteration #$COUNTER"
            cleanup "$CMD_PID"
            exit 1
        fi
    done
    
    echo "Command finished. Iteration #$COUNTER complete."
    COUNTER=$((COUNTER+1))
done
