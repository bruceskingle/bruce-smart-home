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

The following standards exist and speak to this problem domain but are not considered suitable for this purpose.

PAS 1878 https://www.openadr.org/assets/PAS1878_RS_20230912_OpenADR%20Unlocks%20Flexibility%20Throughout%20Europe.pdf

Open ADR https://www.openadr.org/assets/OpenADR%203%20and%20DER%20Management_v1.0%20with%20cover.pdf

IEEE 2030.4 https://en.wikipedia.org/wiki/Electric_vehicle_charging_infrastructure_security
