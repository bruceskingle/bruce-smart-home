# Logging in to GloBird

The app needs the email address and password you log in to the GloBird customer portal, [myaccount.globirdenergy.com.au](https://myaccount.globirdenergy.com.au), with.

The app saves both in the profile, and keeps the portal's session in its cache, so that it logs in as rarely as it can: once, and again only when GloBird ends the session. If a login fails, e.g. because GloBird asks for a captcha, the app does not try again until you log in on this page, so that it cannot lock your account. If GloBird asks for a captcha, log in once on GloBird's web site, then try again later.

If GloBird refuses the saved login, e.g. because you changed the password, the app keeps the login, so that this page is filled in with it, stops polling GloBird and shows this page at once, saying why, until you log in again. Home's pages say so too, with a link here.

**Log In**, the last page in the list once you have logged in, shows this page again, so that you can give a changed password at any time. Logging in replaces what was saved. ![Show](../icons/show.svg) beside the password shows what it holds, e.g. to check the saved one.

Your email address and password only go to GloBird, the app has no server of its own.
