# Dreamscroll Development Log

## 2026-01-07

Goals for today:

- [X] Setup wireguard and validate working end to end
- [ ] Setup Git daemon on VM
- [ ] Create Github repo
- [ ] Register domain??
- [ ] Get dreamscroll working on GCE instance (at least at proof of concept level)

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
