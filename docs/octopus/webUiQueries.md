[< Octopus Energy Plugin](index.md)

# Web UI Queries
This section describes the queries made by the Octopus Web UI [https://octopus.energy/dashboard](https://octopus.energy/dashboard). While this is purely internal implementation detail which may change at any time, it's informative to look at as a way of understanding how to navigate to the information you may want to extract via your own API calls.

The [Chrome Inspector](../howto/ChromeInspector.md) page describes how you can use the Inspector function of the Chrome browser to observe these queries for yourself.

 ## Redaction
 Some of the values in the examples below, such as account numbers and meter IDs have been redacted. In some cases there is documentary evidence of what the actual type of the returned values are (e.g. the GraphQL schema defines the `number` attribute of `AccountInterface` to be `String` (interestingly not `String!` although one imagines that this is not an optional attribute)), in other cases there is not. There *appears* to be further structure in some values, e.g. Account numbers appear to have a hyphen in the second character. In order to aid recognition when comparing examples here to actual returned values some of this apparent structure may be represented in the redaction, so a redacted account number may be shown as `A-B1C2D34E`. This apparent structure *cannot* be relied upon, when writing code you should code against the official schema so an account number should be stored as an unbounded optional (nullable) string value despite the fact that all observed values appear to be of fixed length with some internal structure.

## Trying These Calls For Yourself
For each call in the following section we show the `Query` and `Variables`, these values can be copied and pasted into https://api.octopus.energy/v1/graphql-playground to execute the query interactively. The official API documentation contains a link to https://api.octopus.energy/v1/graphql which is another very similar, but apparently older, version of the same thing.

Before you can do this you need to authenticate (log in), this can be done using your email address and password, or API key. I strongly recommend *not* storing your password anywhere including interactive GraphQL UIs, so the first step is to find your API Key.

This is shown at https://octopus.energy/dashboard/new/accounts/personal-details/api-access

![alt text](../howto/APIkey.png)

If you click on the API key it will be copied to your clipboard.

Now navigate to https://api.octopus.energy/v1/graphql-playground, you should see this:

![alt text](../howto/GraphQlPlayground.png)

Paste the following query into the main panel on the left where it says `# Write your query or mutation here`

```gql
mutation obtainKrakenToken($input: ObtainJSONWebTokenInput!) {
  obtainKrakenToken(input: $input) {
    payload
    token
  }
}
```

Now click on `QUERY VARIABLES` at the bottom and paste the following into the panel which opens below, replacing the APIKey placeholder with your key:
```gql
{
  "input": {
    "APIKey": "redacted_api_key_AAAAAAAAAAAAAAA"
  }
}
```

Now click the big play button in the centre and on the right hand side you should see a response similar to this:

```gql
{
  "data": {
    "obtainKrakenToken": {
      "refreshToken": "redacted_refresh_token_pZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtp",
      "refreshExpiresIn": 1725024454,
      "payload": {
        "sub": "kraken|account-user:2402938",
        "gty": "API-KEY",
        "email": "dan@archer.org",
        "tokenUse": "access",
        "iss": "https://api.octopus.energy/v1/graphql/",
        "iat": 1724155372,
        "exp": 1724158972,
        "origIat": 1724155372
      },
      "token": "reacted_jwt_token_3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZW"
    }
  }
}
```
This is a [JSON Web Token (JWT)](https://jwt.io/) which is a short lived bearer token (i.e. a token, possession of which is considered as proof of identity). Although this is a short lived token you should treat it as sensitive and not store it anywhere unnecessarily. If you paste the `token` value into the JWT website above you will see the contents and as you will see, the payload of the JWT is also provided in the GraphQL response as the `payload` attribute as a convenience.

I believe the jwt.io site is safe and does not copy the tokens pasted into it, but cannot guarantee this. Extreme caution should be used with other sites which offer similar functionality.

The `iat` attribute is the time when the token was issued, if you paste the value `1724155372` into https://www.epochconverter.com/ you will see that this is a Unix timestamp (starting from 1st January 1970) in seconds for Tuesday, 20 August 2024 12:02:52.

The `exp` attribute is the expiry time for the token, in this case `1724158972` or Tuesday, 20 August 2024 13:02:52, so we can see that the token is valid for a maximum of 1 hour.

If you are writing an application to call the API you will need to refresh the token each time before it expires. Using the graphql-playground you will need to repeat the process above and update the token as and when it expires.

Once you have a token, click on the `HTTP HEADERS` tab at the bottom left of the window and paste in this, replacing the token placeholder with the value you just received:

```gql
{
  "Authorization": "reacted_jwt_token_3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZWxsLWtub3duL2p3a3MuanNvbiIsImtpZCI6InF3ZW"
}
```

For each of the queries shown below you can try for yourself by pasting the `Query` and `Variables` into the graphql-playground as we did above. In the variables section any placeholder of the form `AAAAA` or `99999` must be replaced with the appropriate value for your account.



[My Account Page Queries >](webUiQueriesDashboard.md)