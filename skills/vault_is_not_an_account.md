A credentials file on disk is not proof a live account exists. Load BEFORE any login, publish, or signup dispatch that assumes a stored credential record means there is something to log into.

# vault_is_not_an_account — credentials on disk are not an account

Work gets dispatched against a vault entry the way it gets dispatched against
a real account, because both look identical from the workspace: a JSON file
with a username and a password. The account behind one of them was never
created — the signup died at a CAPTCHA. Every retry then burns an episode
re-discovering a wall that was already recorded twice.

## When to use
- Before any login, publish, post, or signup dispatch that depends on a stored
  credential record.
- Before scheduling recurring work on a channel whose account status is not
  confirmed by a successful server-side action.

## Procedure
1. Read only the non-secret fields: `status`, `notes`, `provider`, and the
   public identifier (address / email / username). NEVER print or log a
   password, token, secret, key, seed, or auth field — not into episode notes,
   not into an error message.
2. If `status` contains `blocked`, `not-registered`, `captcha`, `recaptcha`,
   or `turnstile`, there is NO server-side account. PARK the work. Do not
   attempt a login, do not retry the signup, do not dispatch the publish.
3. If `status` is missing, treat the entry as unverified: run ONE read-only
   login probe, record the exact error verbatim in `status`, then park or
   proceed on that evidence.
4. Check the provider field against what the file is named for. A file named
   for one network can hold credentials for a different one entirely; the name
   is a label, the provider field is the fact.

## Pitfalls
- A finished, audit-clean draft for a channel proves the draft is good. It
  proves nothing about whether the account can publish it.
- A human-interaction wall (CAPTCHA checkbox, phone verification) does not
  become passable by retrying with a different HTTP client. Both a plain HTTP
  client and a real browser driver hitting the same CSRF/session wall is
  confirmation, not a reason for a third attempt.
- A wall recorded twice is a wall. Park it with a date and stop re-probing.

## Verification
The account is live only when a read-only authenticated call to the provider
succeeds and that success is written back into `status` with its date. Until
then the lane is PARKED, and anything scheduled against it is cancelled rather
than retried.

## OUR INSTANCE
Record here: the vault directory, one line per credential file with its
provider, current `status`, and the date of the last probe — including the
known walls that must not be re-probed and which entry is the working sender.
