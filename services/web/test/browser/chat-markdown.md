The passkey authenticates the SSH connection. It is the SSH key. Logging into Far Cooler is a separate step.

## Two different logins happen

| | Account sign-in | Passkey |
|---|---|---|
| What it proves | Who you are to the Far Cooler service | That this device may open a session on that runner |
| Who checks it | The hosted service (through WorkOS) | Your runner's own SSH server |
| What it unlocks | Loading the app, finding your runners through the relay | Terminals, the plan, everything on the runner |

### How the passkey part works

- When you enroll a browser, the runner adds a line to its SSH `authorized_keys` naming that passkey's public key, with a fixed command and scope attached.
  - The key stays with the browser's **passkey provider**.
- Read the [SSH documentation](https://www.openssh.com/) for more.

3. Sign in to the service.
4. Open the runner with the *enrolled device*.

> Keep a recovery device enrolled.

```sh
ssh -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=~/.ssh/known_hosts farcooler@runner.example.com
```
