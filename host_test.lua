-- Client
-- #!/usr/bin/env lua5.1

local wait_count = 10 -- in seconds

local dummy_hash = "YZ0123"
local mm = require("matchmaker")

mm:init(dummy_hash, '127.0.0.1', 3000, 1)

if mm:check_config() == false then return end

-- create_session() creates a new session on the server
-- create_session(true) creates a private session
mm:create_session()

-- wait until we get our unique session key (secret)
while(mm:get_session():len() == 0) do
    mm:poll()
end

print("Server returned session code: "..mm:get_session())

while(wait_count > 0) do
    wait_count = wait_count - 1
    print("wait_count: "..wait_count)
    mm:poll()
    mm:sleep(1.0)
end

local remote_addr = mm:get_remote_addr()

if remote_addr ~= '' then
    print("joined session with remote "..remote_addr)
    
    -- use the socket when connection is available!
    -- mm.socket 
else 
    print("No one joined the session")
end

-- cleanup
-- will also close session on server for us
mm:close()

print('Done')