# Logging in to the Powerwall

The app logs in to the gateway as its customer, the same as the gateway's own web page on your home network:

- **Gateway Address**, its IP address on your home network. Leave it empty and the app searches the network for it. If the gateway's address changes later, the app finds it again.
- **Email** and **Password** of the customer login. If the password has never been changed, it is the last 5 characters of the password on the label inside the gateway's door.

The app saves the email and password in the profile, so that it can log in again when the gateway asks it to. They only go to the gateway.

**Log In**, the last page in the list once you have logged in, shows this page again, so that you can give a changed gateway address or password at any time. Logging in replaces what was saved. ![Show](../icons/show.svg) beside the password shows what it holds, so that you can check it.

Check the password before logging in again after a failure. The gateway locks out the customer login for a while after repeated failures, which stops other programs which use it too, such as Home Assistant.

If the gateway refuses the saved login, e.g. because its password was changed, the app keeps the login, so that this page is filled in with it, stops polling the gateway and shows this page at once, saying why, until you log in again. Home's pages say so too, with a link here.
