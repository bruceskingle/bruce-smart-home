# Logging in to myenergi

The app needs your hub's serial number and an API key to fetch your data from myenergi:

- **Hub Serial Number**, on the label of the hub, and in the myenergi app.
- **API Key**, which you make on the myenergi web site, [myaccount.myenergi.com](https://myaccount.myenergi.com), not in the app: under *Products*, choose the hub, then *Advanced* (the key icon), *Generate new API key*. It must be the account the devices were registered with. The key is only shown once, and only one can exist at a time, so making one stops any other program which uses an older key.

**Log In**, the last page in the list once you have logged in, shows this page again, so that you can give a changed hub serial number or API key at any time. Logging in replaces what was saved. ![Show](../icons/show.svg) beside the API key shows what it holds, e.g. to check the saved one.

The app saves both in the profile. If myenergi refuses the saved login, e.g. because a new API key was made, the app keeps the login, so that this page is filled in with it, stops polling myenergi and shows this page at once, saying why, until you log in again. Home's pages say so too, with a link here.

They only go to myenergi, the app has no server of its own.
