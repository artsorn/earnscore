# Recovery and finalization

Recovery is intentionally narrow: it starts from matches that were admitted
while Live and whose result is still unresolved. It never crawls the finished
or historical list.

## State flow

```text
LIVE -> RECOVERY_PENDING -> LIVE
                         \-> FINISHED/CANCELLED/POSTPONED/ABANDONED
                              -> FINAL detail jobs -> FINALIZED
                         \-> UNKNOWN (grace expired)
```

The feed lifecycle hook records a transport disconnect. Parser, filter and
source-contract failures are not completion signals. On reconnect/startup the
recovery candidate query requires live history, excludes terminal/finalized
matches, and creates an idempotent `recovery_jobs` row.

Each recovery job has a lease, attempt count, retry schedule and grace expiry.
Expired leases are returned to `FAILED_RETRYABLE`; a not-found result is retried
until grace expires, then the existing match is marked `UNKNOWN`. No replacement
match is created.

## Finalization

An offline terminal result creates one `FINAL` version and one phase lock for
the match. The required refresh is `overview`, closing `odds`, `stats`,
`incidents`, `lineups` and materialized `period_scores`. The lock prevents
`INITIAL`, `FINAL` and `MANUAL` jobs from being claimed concurrently. The
version becomes immutable only after every required job is complete or
explicitly empty/permanent. Replaying recovery sees the completed version and
does not run the final refresh again.

An operator force action must call the audited `MANUAL` version path with an
actor and reason. It creates a new version and audit row; it never edits the
completed recovery version in place.

## Safe operations

- Stop/restart the process; expired recovery/detail leases are reclaimed on the
  next startup.
- Inspect `recovery_jobs`, `finalization_versions` and `recovery_audit` on a
  temporary database before retrying an incident.
- Retry only the existing recovery job. Do not insert a new canonical Match
  for a not-found response.
- If disk/database repair is required, use the verified backup/restore flow
  before starting the feed again.

The finalizer writes no source URL, raw feed payload or browser target into
recovery metadata. A detail URL is reacquired from the current detail worker
input after restart.
