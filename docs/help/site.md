# Site

Home's Site page says where in your house each module measures, its **topology**, from the grid connection down, so that the [Electricity](electricity.md) page can split the electricity you import from the grid into the car's, the batteries' and the house's. It differs from house to house: a Tesla gateway's grid clamp is usually fitted where the grid comes in, so it measures the car's charger too, but some houses have it moved so that the car is outside it. The app can not tell which yours is, so you say. Until you do, the import is shown by the rate it was billed at only.

## The diagram

The diagram draws the grid on the left, and what each circuit feeds to its right, in the colour of what it is for: grey for the grid, white for a board or gateway, blue for the car, purple for a battery, yellow for solar panels and green for the house. Under each circuit's name are its **channels**, what measures it, e.g. `tesla:site`, the gateway's site meter. A circuit with no channel says **the rest**: it is what feeds it less what its other circuits measured, e.g. a car beside the gateway has the grid's import less the gateway's.

## Editing it

Until it is set, the page shows a first topology made from your modules' channels: the grid, measured by your supplier's meter, the Tesla gateway under it with the Powerwalls, the solar panels and the house. **Start from this** lets you change it. Where your car's charger is only you know: if it is outside the gateway, **Add under** the grid a circuit for the car, with no channel.

Each row is a circuit, indented under the circuit which feeds it:

- Its **name**, shown in the diagram.
- What it is **for**: the grid (only the top circuit), a board or gateway which feeds others, the car, a battery, solar panels, or the house.
- **Measured by**: click a channel to put it on the circuit or take it off. Its tooltip says what it is. The channels are listed under the diagram too.
- **Add under** adds a circuit fed from it, and **Remove** removes it, with everything under it.

Only one of a circuit's circuits can be without a channel, as how the rest is shared between two can not be told, and a channel can only measure one circuit. The page says what is wrong in red, and **Save** waits until it is right. **Undo changes** goes back to what was saved, and **Clear** forgets the topology, so the import is shown by rate only again.

## Your channels

- **Octopus**, `octopus:electricity`: the meter Octopus bills from, its half hourly readings, and, if you have one, the [Home Mini's](octopus/home-mini.md), which reads the same meter every 10 seconds.
- **GloBird**, `globird:electricity`: the meter GloBird bills from, every 5 minutes.
- **Tesla**, read by a [capture device](devices.md): `tesla:site`, the gateway's site meter, its grid clamp; `tesla:battery`, the Powerwalls; `tesla:solar`, the solar panels; and `tesla:load`, the house, as the gateway works it out from the others.
