A simple lightweight bot software for Second Life and OpenSimulator.
Lightweight replacement for .NET/Mono bot software.

First version follows only one single simple scenario:
* Connect to the grid, connect to the region
* If current region is not the target region, issue teleport command to the target destination
* Issue "sit" command on a target object.
* Keep the avatar alive, respond all necessary server queries to keep it online.

It requires one single config file to run with following parameters:
* Grid URL (Second Life Agni by default)
* User name
* Password or password hash if grid can sign in by hash. In case of hash we need a separate tool for hash generation
* Region name and coordinates for teleport command
* UUID of the object to sit on
