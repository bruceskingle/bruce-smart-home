# Logging in to Octopus

The app needs to log in to your Octopus account to fetch your data. There are two ways:

- **Login with Email and Password**, the same as for the Octopus web site and app.
- **Use API Key**, a key Octopus gives you for programs like this one. Find it in your Octopus account online under *Personal details*, *Developer settings*, see [Getting started](../../gettingStarted/index.md).

With email and password, the app saves the login token Octopus gives it, not your password. With an API key, it saves the key in the domain. Either way it stays logged in. If Octopus refuses the saved login, e.g. because the API key was changed, or the token has expired, the app removes the token, which is no use, but keeps any API key, so that this page is filled in with it, stops polling Octopus and shows this page at once, saying why, until you log in again. Home's pages say so too, with a link here.

**Log In**, the last page in the list once you have logged in, shows this page again, so that you can give a changed password or API key at any time. Logging in replaces what was saved. ![Show](../icons/show.svg) beside the password or API key shows what it holds, e.g. to check the saved key.

Your login only goes to Octopus, the app has no server of its own.
