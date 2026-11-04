# GloBird portal test data

Replies of the GloBird Energy customer portal (`https://myaccount.globirdenergy.com.au`) to the calls the GloBird
module makes, from a real account in New South Wales with solar, Powerwalls and a zappi, on a time-of-use plan, for
developing and testing without logging in.

Every identifier is replaced by a keyed hash of it of the same shape, digits for digits and letters for letters, the
same wherever the same original was, and names and addresses by made up ones, e.g. `1 Example Street, Sydney NSW
2000`, and it was checked that none of the originals is left. The account's dates, of its invoices, payments and
letters, are all moved back by the same number of days, and its amounts of money all scaled by the same factor, so
that they still match one another, e.g. the balance is the latest invoice, but are not the real ones. The usage is cut to three NEM days, 1 to 3 November
2026, and scrambled: each hour is the same hour of a day picked at random from five weeks of the account's readings,
scaled by up to 15% either way, so a day's shape, the off-peak charging, the empty peak and the solar export, is
real, but no reading or day is. Each row's `usage`, and the costs of usage and solar, are worked out again to match.

| File | Call |
| --- | --- |
| `currentuser.json` | `GET /api/account/currentuser`: the accounts and their services, power and gas |
| `balance.json` | `GET /api/transaction/balance?accountId=` |
| `dashboard.json` | `GET /api/account/dashboard?accountId=`: balance, the latest invoice, recent transactions |
| `signupinfo.json` | `GET /api/account/getSignupInfo?accountId=`: empty for this account |
| `readmeters.json` | `GET /api/site/readmeters?accountServiceId=`: the meters, one removed, one energised |
| `usage.json` | `POST /api/site/accountservicetimezonesmartmeterread?accountServiceId=`: 5 minute readings |
| `cost.json` | `POST /api/transaction/CostDetail`: each day's cost by category |
| `accountservicestatus.json` | `GET /api/site/accountservicestatus`: `success` false for this account |

What the usage is like, see [docs/PowerSchema.md](../../docs/PowerSchema.md):

- A row for each day, meter register (`suffix`: `E1` import, `B1` export) and charge type (`Peak Usage`,
  `Offpeak Usage`, `Standard Solar Feed in`), with `usageArray`, the day's 288 five minute readings in kWh, from
  midnight NEM time (UTC+10 all year), and `usage`, the charge type's total.
- The same day's whole `usageArray` is repeated on each of a register's charge type rows; `timePeriods` says which
  times of day are the row's, and `usage` is the sum over them.
- Costs are a row for each day and `chargeCategory`: `USAGE`, `SUPPLY`, `SOLAR`, `ZEROHERO Credit`,
  `Super Export top up`, with the `amount` in AUD and the `quantity`, kWh for usage and solar.
