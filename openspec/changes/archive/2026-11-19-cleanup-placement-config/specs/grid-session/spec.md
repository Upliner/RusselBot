# Spec Delta

## MODIFIED Requirements

### Requirement: Login to the configured grid
The bot SHALL log in to the grid at the configured login URL with the configured user and password hash, over HTTPS when the URL is `https` and plain HTTP when it is `http`. A plain `password` from the config SHALL be converted to the hash form before sending. The login request SHALL ask to start at the configured region and position when the config names them; otherwise it SHALL ask for the avatar's home location when `start` is `home`, and for the avatar's last location when `start` is `latest` or absent. The bot SHALL identify itself with its own client name and version and SHALL NOT claim to be another viewer. The bot SHALL follow a login server's instruction to continue the login at another URL.

#### Scenario: Successful login to OpenSim
- **WHEN** the bot is started with valid credentials for a running local OpenSim
- **THEN** the avatar appears online in a region of that grid within 30 seconds

#### Scenario: Login lands directly in the target region
- **WHEN** the target region is online and the grid honours the requested start location
- **THEN** the avatar's first region is the target region

#### Scenario: Requested start location not honoured
- **WHEN** the grid places the avatar in a different region than requested
- **THEN** the login still counts as successful and the session is established in the region the grid chose

#### Scenario: Login at the last location
- **WHEN** the config has neither `region` nor `start`, and the avatar was last in region `Home`
- **THEN** the login request asks for the last location and the avatar's first region is `Home`

#### Scenario: Login at home
- **WHEN** the config has `start = "home"` and the avatar's home is in region `Target`
- **THEN** the login request asks for the home location and the avatar's first region is `Target`
