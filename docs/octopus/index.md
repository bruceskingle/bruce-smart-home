[< Documentation Home](../index.md)

# Octopus Energy Plugin
Information relating to the Octopus Energy API and how Bruce's Smart Home uses it.

Octopus provides two APIs REST and GraphQL. Bruce's Smart Home uses the GraphQL API.

Octopus Energy provide some [Official API docs](https://developer.octopus.energy) which should obviously be treated as definitive, anything here is an external attempt to clarify and explain by external introspection. The official documentation appears to be largely generated which has the advantage that it is correct (or at least reflects the implemented reality), but this does not necessarily help to explain what everything actually means very well. The documentation here is largely written as an aid to writing the Bruce's Smart Home Octopus plugin, but I hope it may also be useful to others who may be trying to use the API.

## General Observations
There are a lot of deprecated attributes which have alternative newer implementations, in these cases I have ignored the deprecated versions and used the replacements.

When GraphQL Interface objects are returned the `__typename` attribute contains the name of the underlying concrete type. A query can use inline fragments to request attributes which exist in the underlying concrete type but not in the interface, but there are cases where multiple identical types implement an interface with the same set of attributes, e.g.
`Charge`,
`Payment`,
`Refund` and
`Credit`
all implement `TransactionType`
and all have the same attributes except for Charge which has two additional values.

The following inline fragment may be used to access these in a query, but you will also likely see the `__typename` attribute being requested because this tells you what type of transaction is happening.

```gql
... on Charge {
    consumption
    isExport
}
```

[Meter Consumption](meterConsumption.md) Observations about the ```consumption``` query on ```ElectricityMeterType```

## Plugin Details
The following pages provide further details about this plugin.

[Web UI Queries](webUiQueries.md) shows details of the queries made to the GraphQL API by the Octopus Energy Account Dashboard application.

[Prototype Queries](prototypeQueries.md)
shows details of some queries which might form the basis of a GraphQL application.

## HowTos
If you are new to this the following HowTos may be helpful:

[GraphQL Playground HowTo](../howto/GraphQL.md) Provides an introduction to GraphQL and shows how to use the GraphQL Playground UI to make interactive queries.

[Chrome Browser Inspector HowTo](../howto/ChromeInspector.md) Provides an introduction to using the Chrome browser inspector to observe API calls made by web pages.