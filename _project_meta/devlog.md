# Dreamscroll Development Log

## 2026-01-23

Main goal for today is to incorporate structured illumination into the app.

## 2026-01-22

Success on the auth front! Took longer than expect but learned a lot. Now we
need to audit the app and fix up the core functionality as we make the final
push towards hosting on GCE and dogfooding live. Basically, what needs to happen
so that this can run 24x7 (crudely) for dogfood round the clock?

- [ ] Formalize the Illumination structure instead of just relying on simple
  text splitting.
- [ ] Fix up the export/import API so can run against a prod/dogfood instance.

EOD:

- Stubbed out a basic structured illumination via vibe coding and am now tuning
  the prompt.
- Tons of cleanups and reorganizations, notably of Error handling.

## 2026-01-21

Token-based auth is working for the API and our internal representation of a
"user" is now unified across both session auth and API.

## 2026-01-20

Significant progress on API and token-based auth.

## 2026-01-18

Today, intend to make more progress on the API implementation and still need to
migrate the schema so that all captures/etc properly relate to a user.

EOD: API structure is feeling good. The capture model now has a user_id field
but it's not (yet) set during various other flows of the app. TODO.

## 2026-01-17

Now that we have auth and a basic concept of User, need to port the rest of the
app to rely on this. Notably, this means ensuring that capture creation is tide
to a User, etc.

- [ ] Rework capture entity to be a relation of user
- [ ] Ensure import/export work in the context of a specific user id
- [X] Start sketching out API?

EOD: Made good progress on the API implementation but unfortunately it seems a
bit slow via Axum:

- Fetching all captures, but returning just 1 takes: 16ms
- Fetching all capatures, returning them all takes: 60ms

Where does all the extra time come from?

## 2026-01-16

Took a long hiatus to focus on family and job interviews. Back in the saddle
today.

Did play around in the `auth` branch for a day or two, but I think I was biting
off too much at once by trying to think simultaneously about both web UI auth
and API auth, using two different crates at different levels of maturity.

Going to reset and focus first and foremost on introducing a User entity and web
UI auth and then follow up with API auth separately.

- [X] Get axum-login working end to end.

TODO:

- Currently the route handlers which need auth are pretected in the serving
  layer, but probably they should have some internal boilerplate at the top to
  verify authentication.
- Research session store.
- Possibly just use SSL even for localdev?

## 2026-01-08

One month anniversary of start of project!

Changing gears a bit today, I think we should focus on users, auth, and
converting the controller/ code into a more proper API (both internal app- auth
and token-based REST webservice auth).

- [ ] Investigate best approach for users
- [ ] Investigate bets approach for auth, emphasizing flexible approach that
  supports API
- [ ] Start migrating controller/ over to api/

## 2026-01-07

Goals for today:

- [X] Setup wireguard and validate working end to end
- [X] Setup Git daemon on VM
- [ ] Create Github repo
- [ ] Register domain??
- [X] Get dreamscroll working on GCE instance (at least at proof of concept level)

It works! Now running on a GCE VM instance! Next up, start refactoring and
organizing for a true cloud-shaped deployment, chiefly (1) a non-sqlite database
and (2) a media storage service (like S3).

## 2026-01-06

Starting to sketch out a basic Google Cloud topology of two instances: (1)
e2-micro for managing TailScale topology and (2) for actually hosting the app
self-contained. For now, I plan on simply self-hosting postgres on this box
before I venture into researching the Google RDS equivalent.

Basic Benchmark of building ripgrep clean:

- GCE e2-medium (2 vCPU) on a "standard persistent" disk: 36.44s external (57.80s user)
- Macbook Air M2 (8 cores): 7.44s external (21.86s user)

Given the comparison on user timing, looks like the additional cores on the M2
are having a big effect (i.e. 5x speed up on wall clock but less than 3x speedup
on user cpu time.). Honestly for like $26 a month, the e2-medium is better than
I expected.

Current decision point: do I use Tailscale mesh or is there some Google Cloud
VPN that will magically work? Basically I want the ability to connect my laptop
or phone to my Google Cloud "environment" trivially and access all the resources
there.

## 2026-01-05

Hilariously, getting an API key for Gemini is nontrivial. There are any
marketing and landing pages which are not helpful.

API keys are here:
[https://aistudio.google.com/api-keys](https://aistudio.google.com/api-keys)

The Google Cloud Console for the project is here:
[https://console.cloud.google.com/home/dashboard?project=mdrcode](https://console.cloud.google.com/home/dashboard?project=mdrcode)
