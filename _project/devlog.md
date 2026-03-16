# Dreamscroll Development Log

## 2026-03-16

Initially, the spark quality was quite poor because the LLMs hallucinate the
links! Many links (perhaps even a majority) were simply wrong, and many sites
now return soft 404s - so you click a link expecting article X and you see
completely unrelated article Y. Worked yesterday to enable web grounding which
had surprisingly positive impact on Grok generated Sparks (but unfortunately not
Gemini, which seems strange?). But bottom line, Spark quality is now great!

The next big milestone will be evolving the timeline UI to be card-based so that
we can have a single unified view of both capture cards and spark cards.

## 2026-03-14

Sparks now running fully in the prod backend! Can be viewed on a special debug
page /sparks. Next step is to wire them into the card-based timeline UI properly.

## 2026-03-12

The beginning of a "Spark" flow are now in place in src/ignition, including both
a Grok and Gemini implementation. So far the quality is ... very good.

Next up:

- [X] Add spark models to the database schema, and store
- [X] Async webhook for spark
- [X] API for triggering a spark (takes list of capture IDs)
- [ ] Improve the UI architecture to support extensible "cards", which can be
  either "capture cards" (including an illumination) and "spark cards".
- [ ] Show spark cards within the timeline feed.

## 2026-03-10

Gonna take a break from the UI work and get back to the actual data quality.
Going to rough out the Igniter -> Spark concept which will take a set of
captures (+illuminations) as input and then provide recommendations.

## 2027-03-09

Over the weekend I have migrated from Pub/Sub to using Cloud Tasks for the async
workloads . This is motivated by Cloud Tasks support for low-cost querying of
individual task status which can then be trivially broadcast to the client using
Firebase SDK (hopefully...).

## 2027-03-06

This week saw significant progress on the mobile layout and overall
responsiveness of the app. The content cards on a canvas paradigm has been
solidified, providing a consistent experience across devices.

Next up, a few big things on the horizon:

- Implement a prototype of the "spark" concept, by which multiple illuminations
  can be combined as a prompt-context to generate new insights, actions, and
  follow up. This is the key "second half" of the flywheel.
- Thread real-time "push" updates to the app so the user sees the progress of
  the illumination pipeline. So imagine you upload an image and you see whether
  it is queued or currently being illuminated, and maybe even we project an ETA
  using trailing average of past illuminations.

## 2027-03-05

Tons of Codex-led iteration on mobile and we have a slick, responsive layout.
Not perfect but 10x better than what we had before. We have settled into a
fundamental paradigm of content cards on a canvas. On Desktop the cards are two
column, then they collapse down to single column on mobile.

## 2026-03-04

Using the prod app constantly now, it feels great. Rather urgent needs and
painpoints from a dogfooding perspective:

- [X] A better UI on mobile. There is currently too much whitespace (padding,
etc) and the text is too small. I might spin the vibe wheel on this and see if
Codex can come up with something better.

## 2026-02-27

The app now works in production. dreamscroll.ai is live in prod.

Accomplishments over the past couple weeks:

- Fully Docker-ized container build and deploy
- Full deployment via Google Cloud Run
- Google Cloud Storage working end to end
- Async Illumination working via PubSub
- Illumination now uses Vertex AI endpoint when in prod

Important pending work before we can declare version 0.1 complete:

- [X] Import via API, so can drive imports from local digest to prod
- [ ] Generate alternate resolutions, including thumbnails, of media
- [ ] Signed cookie protection on access to cloud storage bucket
- [ ] OpenTelemetry metrics
- [X] Cached prompts with Vertex AI (apparently "implicit" caching happens automatically?)
- [X] Better UI on mobile web (white space too aggressive at present)

## 2026-02-18

Been a busy week (but neglected to update the log). The app now fully runs
container-ized in preparation for Cloud Run and it works with postgres.

The only remaining blocker to running in Google Cloud is leveraging a pub
sub queue for driving the illuminator workflow. Once that is done, then
the app will be ready for (at least alpha-level) testing in gcloud environment.

Pending backlog:

- [X] Re-architect the illuminator workflow to be webhooks driven by a queue
  (pub sub) invoker.
- [ ] Figure out signed cookie protection for media URLs (since currently) they are
  publicly visible but should be locked down.
- [X] Improve illuminator flow to use Vertex API instead of DMZ API.

## 2026-02-7

Up next:

- [X] Use a proper backing SessionStore for Auth
- [X] Port the config structure over to envy and dotenvy
- [X] Get GCloud storage working end to end

## 2026-02-06

Over the past week, executed a massive refactor to make the API client stateful
so that the DB and Storage handles could be encapsulated inside and hidden
entirely from API clients. Similarly, this work allowed the json serialization
to become stateful (parameterized) which was necessary to serialize different
URLs depending on the different storage backends in use. Basically, the proper
JSON expression became a function of dynamic details like storage provider.

Additionally, integrated with the local Google Cloud Storage emulator (fsouza/
fake-gcs-server) to verify Gloud auth working.

Once this was up and running, spent several days registering a domain
(dreamscroll.ai) and getting it hosted on Google Cloud, which took many
iterations but now works!

The app now runs successfully in GCloud but still uses "local" mechanisms like
file-backed sqlite.

The next step is to get it running in a fully "cloud-native" configuration like:

- [X] Use postgres in the cloud
- [X] Use prod GCloud storage (and possibly sign URLs)
- [X] Investigate using a more durable session store than just in-process-memory
- [X] Possibly this work will necessitate adopting a more flexible config solution
   like dotenvy
- [X] Use Cloud Run instead of manually deploying and running on an unmanaged instance

## 2026-01-29

Introduced a new auth concept of "service context" which is created from a
JWT service token. This is intended to be a security identity for backend/
system services like the illumination worker.

Import/Export roundtripping with preservation of `created_at` timestamp now
works properly.

## 2026-01-28

Successfully factored out social media account extraction to be structured.

Elevated knodes, xqueries, and social_media to be direct relations of capture
instead of hanging off the illumination.

## 2026-01-27

The KNodes and XQueries are beautiful and I am continually surprised by the
quality. However, when attempting to extract social media accounts, frequently
the model gets confused. e.g. FirstName LastName (@cool_handle) will sometime be
reported as any of:

- RealPerson: @handle
- RealPerson: handle
- RealPerson: FirstName LastName
- RealPerson: FirstName LastName (@handle)

So there is a fair amount of randomness here. Today I am going to attempt to
elevate the social media accounts to their own structred response.

## 2026-01-26

Prompt tuning and eval.

## 2026-01-25

Structured Illumination now flows throughout the app and api. It looks
beautiful.

Also, suggested searches (aka "explore queries" or XQueries) and knowledge
entity nodes (aka "KNodes") are also present.

Done for today.

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
