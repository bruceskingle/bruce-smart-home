# Open Consumer Energy Agreement Negotiation

The OCEAN protocol is a proposal for an Open System to facilitate the management of electricity demand from consumer devices such as Home Battery Systems or EV Chargers and their supply by an electricity supplier.

This is a light weight protocol which can be easily added to the existing API provided by an energy supplier to allow its customers to advertise their capacity to accept energy at favorable prices without disrupting the existing operation of either the consumer households or the energy supplier. It is intended that this protocol can be implemented incrementally, in the first instance by intermediating the communication between the supplier and the consumer devices with cheap micro controller based systems while providing for the implementation of the protocol directly into those consumer device in the long term.

The catalyst for the development of this protocol is the experience of the author with the Octopus Intelligent Go tariff and the implementation of an application to monitor the consumption of electricity in his own home and to manage the most cost effective operation of its electricity consumption.

It is the author's observation that the operation of the Intelligent Go tariff has lead to a number of complications. Whereas the energy supplier wishes to make energy available a cheap prices for the advantage of themselves and their customers, and to manage demand, shifting load from busy to quiet times to reduce costs and the environmental impact of electricity generation. Consumers sometimes act in ways to maximize their advantage from cheap periods, thereby generating unforeseen exceptional demand which exceeds the energy purchased on the wholesale markets leading to financial losses for the company.

OCEAN has been devised as a mechanism to manage this relationship in such a way that these problems could be eliminated to the advantage of both the supplier and consumer.

## Approach

It is assumed that the energy provider supplies the consumer under a smart tariff, where there are at least two rates (prices) for energy supply. The consumer is assumed to have one or more devices whose energy demand can be managed such as an electric vehicle (EV) charger or a home battery system.

The consumer will signal to the supplier what energy they would consume if offered on favorable terms, and the supplier will decide if and when to offer such terms and with what limits. In the current system the supplier has no real way to assess the size of the demand because of the limitations of domestic AC EV chargers and the fact that they have no visibility on what home battery systems the customer may have.

In an ideal world, in the long term, the technical capabilities needed to make this happen would be built into the consumer devices (EV chargers, battery management systems etc) themselves. In the short term these capabilities will be provided by inexpensive ESP32 micro controller and free open source software operated by the consumer.

In the event that the smart technology on the consumer site fails or is disconnected, the tariff continues to function without unexpected consequences on either party.

In the current Intelligent Go tariff, the user configures the percentage of their EVs full capacity they wish to add and the time by which they wish it to be ready. AC chargers do not have a way of obtaining the current state of charge from the EV which means there is no way to allow the consumer to specify the charge level they wish to achieve, and it seems unlikely that consumers will update the "charge to add" setting on a regular basis, so the supplier is largely in the dark about what the actual demand will be.

Currently all consumption in intelligent charging sessions which the supplier decides to schedule during the daytime (expensive) period is billed at the cheap rate and this motivates consumers to implement home automations to charge their battery systems at those times, and this demand is entirely invisible to the supplier.

With OCEAN the consumer would operate one or more ESP32 micro controllers, which are connected to their home WiFi network and have internet access. These devices poll the home battery systems directly through a local or internet API provided by the battery system manufacturer to assess the current state of charge and therefore the potential demand from the home battery system as well as a CANbus module fitted to any EVs in the household to establish the state of charge of any vehicles.

These devices then contact the supplier's servers on a regular schedule advising what extra demand would be made in the event that favorable pricing was offered.

The firmware for these micro controllers is delivered as part of the Bruce's Smart Home software project (which is free and open source).

In the event that the supplier is able to provide energy on these favorable terms, they respond with the time window when these terms will apply and the limit of energy (in kWh) available at those terms. The consumers home automation system then manages the consumption from those devices. Any demand which exceeds the agreed amounts will be billed at the suppliers usual rate for that time of day. Any consumer which significantly under consumes the agreed quantity can be removed from the tariff by the supplier.

It is important to note that the protocol is stateless, a pseudo code representation of the conversation might look like this:

| Time  | Consumer     | Supplier                    | Notes                                                                                                                                                                                 |
| ----- | ------------ | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 09:55 | Demand 5     | OK                          | Consumer demand would be 5 kW for 1 30 minute period (2.5 kh), supplier makes no offer                                                                                                |
| 10:25 | Demand 5 5   | OK                          | Consumer demand 5 kW for 2 consecutive periods (assume the battery is discharging at that rate at the same time so now has more capacity, 5kW is the limit on how fast it can charge) |
| 10:55 | Demand 5 5 5 | Supply 7.5kwh 11:00 - 13:00 | Consumer now has capacity for a total of 7.5 kWh over at least 90 minutes, supplier offers to provide up to 7.5 kWh within the given time window                                      |

The consumer device attempts to make requests shortly before the start of a billing period, the supplier makes a time and energy limited offer at preferential rates. The consumer then makes additional demand and is billed appropriately. If the demand exceeds what the supplier agreed then it is billed as if it this system did not exist so the supplier is protected for excessive unforeseen demand. If the demand is less than agreed the supplier can remove the customer from this tariff, but the consumer is not exposed to and immediate financial risk.

## Proof of Concept

The author will implement both the consumer and supplier ends of this protocol using the ESP32 devices described. The consumer end of the protocol will work exactly as described here, the supplier end of the protocol will use knowledge of the standard hours for cheap and standard rates on Octopus Intelligent Go and flex charging messages from the Octopus API to anticipate when the cheap rate is available to provide the appropriate responses.

For indicative costing purposes the following devices are being used:

| Device                 | Description                             | Retail Price, single unit delivered to the UK | URL                                                   |
| ---------------------- | --------------------------------------- | --------------------------------------------- | ----------------------------------------------------- |
| Waveshare 1.47 ESP32S3 | Cheap option with 1.47in colour display | £12.19                                        | https://www.aliexpress.com/item/1005009260807401.html |
| Waveshare 7.0 ESP32S3  | With 7in Colour touch screen display    | £38.09                                        | https://www.aliexpress.com/item/1005010074374675.html |
| Waveshare 4.2 ESP32S3  | 4in Reflective LCD disply with battery  | £37.09                                        | https://www.aliexpress.com/item/1005012122886937.html |

These devices all require a standard USB power supply and a micoSD memory card.

## Existing Standards

The following standards exist and speak to this problem domain, but are not considered suitable for this purpose. They
share one approach: a supplier, aggregator or network operator controls or instructs the consumer's devices, which
must themselves implement the standard and usually be certified to it. OCEAN takes the opposite approach: the
consumer says what they would take, the supplier offers a limited amount on better terms, and the consumer's own
system decides what to do. None of these standards expresses that exchange, an offer capped in kWh with anything over
it billed at the normal rate, without extensions of its own, at which point little is gained from using it. They are
not rivals, though: a consumer's system could speak one of them to its devices, or to an aggregator, and OCEAN to its
supplier.

### A lesson from the 1990s

This has happened before. In the 1980s and 1990s there were two ways to build a network of computers. One was the
work of the international standards bodies: the ISO's seven layer model of Open Systems Interconnection, X.25 for the
networks themselves, and X.400 for electronic mail, from the CCITT, the committee of the national telephone
administrations. It was thorough, carefully layered and, by the standards of its day, sophisticated, and governments
backed it: the British and American governments both required it in what they bought. The other was TCP/IP, which grew
out of research networks, and was simple, incomplete and informal by comparison, its standards written by whoever
turned up with "rough consensus and running code".

The ISO approach was top down. X.400, for example, divides the world into Administration Management Domains, run by
the national telephone authorities, and Private Management Domains, which connect through them. That is a statement of
who is in charge, not a technical necessity, and it has no place in a technical standard. It assumed that the
network belonged to the authorities and that everyone else was a subscriber to it.

TCP/IP made the opposite assumption. Its network was deliberately simple, and the intelligence was at its edges, in
the computers of the people using it, who could build on it without anyone's permission. Today every one of us uses
TCP/IP every day, and few people under thirty have heard of X.25. The parts of the ISO work which lasted, such as the
X.509 certificates which secure the web, survived by being adopted into the internet, not the other way round.

The standards for managing household energy are, for all their merits, in the tradition of X.400. They are written
from the point of view of the supplier, the aggregator and the network operator, who instruct the household's devices,
and they bring the weight and certification of a system designed for those who run it. That they now travel over
HTTP and JSON does not change who they are written for. OCEAN is in the tradition of TCP/IP: a small, open protocol
which anyone can implement, which leaves the decisions with the household at the edge of the system, and which asks
only that each side keep its word. The internet shows which of those two approaches is more likely to be adopted, and
to grow into things nobody planned.

### PAS 1878

https://www.openadr.org/assets/PAS1878_RS_20230912_OpenADR%20Unlocks%20Flexibility%20Throughout%20Europe.pdf

PAS 1878 is the British Standards Institution's specification for energy smart appliances: how an appliance, a
customer energy manager and a demand side response service provider work together, with requirements for security,
interoperability and protecting the grid. It is aimed at the makers of appliances, which must be built and certified
to it, and at the service providers which then adjust their load. It does not help the households of today, whose
batteries and chargers were not built to it, and it gives the service provider the control, where OCEAN leaves it
with the consumer. It describes how load is managed, not the commercial terms on which a consumer might choose to
take more energy.

### OpenADR 3

https://www.openadr.org/assets/OpenADR%203%20and%20DER%20Management_v1.0%20with%20cover.pdf

OpenADR, from the OpenADR Alliance, is a long established protocol for demand response, and version 3 is a modern
REST and JSON API: a server, the utility's or aggregator's, publishes programs and events, such as prices or requests
to reduce load, and the client at the consumer's site reports back. It is a general framework for a utility to signal
many kinds of resource, so it is broad, and products claiming to support it are expected to be certified through the
Alliance. Its model is one of the server signalling and the client responding; OCEAN's offer of a capped amount of
energy, in answer to the demand the consumer advertised, and the commercial rules around it, would have to be added
as extensions of its own, so a supplier and a consumer would gain little interoperability from it in return for its
weight.

### IEEE 2030.5

https://sagroups.ieee.org/2030-5/

IEEE 2030.5, which began as Smart Energy Profile 2.0, is used, for example in California, for utilities to communicate directly with distributed energy resources such as solar inverters and
batteries. It defines a large data model and a full certificate infrastructure which each device must implement, and
it is built for a utility to monitor and control those devices. That suits a regulated market requiring it of every
inverter, but not a lightweight, voluntary agreement between a supplier and a household using the devices it already
has.

## Related Policy: AI Vision for Energy

https://api.startupcoalition.io/u/2026/11/AI-Vision-for-Energy.pdf

Startup Coalition's response, in November 2026, to the Department for Energy Security and Net Zero's call for evidence
on its vision for an AI enabled clean energy system, makes 20 recommendations. Its central argument is that household
flexibility is held back by access to data and markets, not by technology. OCEAN is a small, open and concrete answer
to several of the problems it describes:

- **Standard flexibility products and a common API** (recommendation 12): it asks that a provider of flexibility be
  able to integrate once and offer its service to every buyer, among network operators. OCEAN is the same idea between
  a household and its supplier, and much lighter.
- **Machine readable tariffs** (recommendation 14): an OCEAN offer, a time window, a limit in kWh and a rate, is a
  tariff a device can act on without scraping a web page.
- **Herding** (recommendation 15): it warns that many automated systems responding to the same price can create new
  peaks. This is the problem OCEAN was devised to solve: the supplier never commits more energy to a household than it
  has offered, so the demand it has to buy is known in advance.
- **Licensing of load control** (recommendation 16): it argues that the Smart Secure Electricity Systems regime should
  license the party which decides how load is adjusted. Under OCEAN that is the household, as the supplier only makes
  an offer, which may keep a consumer's own system, and free open source software such as this project's, outside
  load control licensing, though this would need checking against the regime's final rules.
- **Agents acting for households** (recommendation 17): it asks for authorised agents to act on a household's behalf,
  read access first, then action, with graduated autonomy. An OCEAN device is such an agent, acting only within an
  offer it has been given.
- **Safe autonomy** (recommendations 19 and 20): it asks for safe fallback modes and bounded autonomy. OCEAN's tariff
  carries on as normal if the consumer's system fails, its offers are bounded by their limit in kWh, and the protocol
  is stateless.

It also differs from OCEAN in places. Its proposals for open energy data concern a household's past consumption,
joined to the equipment behind its meter, whereas OCEAN shares only the demand a household chooses to advertise, ahead
of time. And its proposed markets for constrained parts of the network (recommendation 13) could reach households
through OCEAN without any change to it: a supplier could make offers only to households behind a constraint, to use
wind power which would otherwise be curtailed.
