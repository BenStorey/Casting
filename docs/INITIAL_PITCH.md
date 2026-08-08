I want to start a project codenamed "Casting" - which is essentially an agent orchestration tool used for building software. It's aimed at people who are not necessarily technical experts and they just want to guide the software as it gets pulled together automatically by a team of agents working together in a sensible heirarchy.

The idea is that I should be able to do "cast run" - and have the tool start a webservice and give a login/password that the user can use to connect to it with. The human "owner" or "CEO" or whatever name we use will (at least at first) only really work with the "Project Manager" and will guide them on what is important and what needs to be built. They will be able to expand the team (or 'cast') by bringing in "consultants" who are an expert at a particular problem (such as testing, UX design, optimisation etc).

I want there to be a "fun" side of this to make it relatable and interesting - for example I want each consultant to have a name / role / picture and maybe even CV. But this is only a layer to keep things fun and interesting, the core product itself is solving the issues of:

1. How do we allocate work across multiple agents and manage their communication?
2. How do we track / store the history of decisions and changes over time?

Ultimately we will need a dashboard and UI to give real-time access to changes that are happening, to see past events and to see a distributed Jira-style task board. We probably don't want to actually use Jira for this - but we'll want a shared view on what is happening and next up.

The PM will be in charge of deciding who works on what and in what order - taking into account the speed the owner wants to move at, and potentially budget concerns (since we'll be burning real money on tokens here).

The PM will need a way to communicate with the owner - ideally over Telegram or Whatsapp or a web interface - when the owners opinion is necessary before making further progress. The owner should be able to communicate with the PM exactly how much feedback they want, and how happy they are for the PM to make bigger architectural decisions.

The "consultants" could have their own minions (sub-agents), and they should be allowed to pass back feedback upstream to the PM for prioritisation. For example they could spot a refactor or a potential problem - "I noticed https isn't enabled here, I won't fix it now, but should we do it later?". The PM could respond asking for more information. These message events would be displayed to the user as some sort of email communication (so maybe they can all have a [name@projectidea.com](mailto:name@projectidea.com) address or something for fun).

I believe that multiple agents coding together is going to be the future of software development, and I'd like to build a product that attempts to streamline that future, whilst making it fun and enjoyable for the human owner/director as well.

Personally I'm an experienced software developer with 20+ years experience across the entire tech stack, so will be making the big decisions on how this project is written and delivered.



what do you think about this plan?