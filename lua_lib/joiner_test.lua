-- Client
-- #!/usr/bin/env lua5.1

local wait_count = 10 -- in seconds

local dummy_hash = "YZ0123"
local mm = require("matchmaker")

mm:init(dummy_hash, '127.0.0.1', 3000, 1)

if mm:check_config() == false then return end

-- will join a private session by its secret
--mm:join_session("nssiaA1")

-- will join any public session
mm:join_session()

while(wait_count > 0 and not mm:did_join_fail()) do
    wait_count = wait_count - 1
    mm:poll()
    mm:sleep(1.0)
end

print("Join request status="..mm:get_join_status())

local remote_addr = mm:get_remote_addr()

if remote_addr ~= '' then
    print("joined session with remote "..remote_addr)
    
    -- use the socket when connection is available!
    -- mm.socket 
else 
    print("I could not find a session")
end

-- Cleanup
mm:close()

print('Done')