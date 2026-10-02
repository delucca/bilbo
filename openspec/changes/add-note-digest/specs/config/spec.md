# Spec Delta

## ADDED Requirements

### Requirement: Digest settings
The digest keys SHALL be `digest.min_similarity` (a number from 0 to 1, 0.55 by default; the similarity a note's best passage needs to enter the digest when an embedder answers) and `digest.log` (`on` or `off`, `off` by default). Any other value SHALL be an error naming the key.

#### Scenario: Defaults
- **WHEN** the config file sets no digest key
- **THEN** the digest gate uses 0.55 and no digest log is written

#### Scenario: A bad switch
- **WHEN** the config file holds `digest.log = yes`
- **THEN** every verb that reads settings reports an error naming `digest.log`, and `bilbo recall` exits 2
