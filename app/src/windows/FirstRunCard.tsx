import type { ConnectOutcome } from '../ipc'

/** The line above Try again on the popup's card (docs/PUBLIK-INTEGRATION.md §4). */
const KEEP_WORKING =
  'Your checks keep working. When you have linked this computer, press Try again.'

/**
 * The first-run card (docs/PUBLIK-INTEGRATION.md §4), from `publik_connect`'s outcome: the
 * balance line, the cost sentence verbatim, then **Sign in with publik** (**Add a plan or pack**
 * once the computer is linked), which opens the account link by its kind, never a URL. In the
 * popup at $0.00 it has Try again for the pending action; in Settings it has none. Then the own
 * key and the local model, offered equally. As a banner (a balance above $0.00, the action
 * already sent) it shows the first three only.
 */
export default function FirstRunCard({
  card,
  onAccount,
  onChoose,
  onTryAgain,
  banner = false,
}: {
  card: ConnectOutcome
  onAccount: (kind: 'claim' | 'add_credit') => void
  onChoose?: (choice: 'own_key' | 'local') => void
  onTryAgain?: () => void
  banner?: boolean
}) {
  const claimed = card.claim_state === 'claimed'
  return (
    <section className={banner ? 'first-run banner' : 'first-run'} aria-label="publik API">
      <p className="balance">{card.balance_line}</p>
      <p>{card.cost_sentence}</p>
      <div className="buttons">
        <button
          type="button"
          className="primary"
          onClick={() => onAccount(claimed ? 'add_credit' : 'claim')}
        >
          {claimed ? 'Add a plan or pack' : 'Sign in with publik'}
        </button>
      </div>
      {!banner && onTryAgain && (
        <>
          <p>{KEEP_WORKING}</p>
          <div className="buttons">
            <button type="button" onClick={onTryAgain}>
              Try again
            </button>
          </div>
        </>
      )}
      {!banner && onChoose && (
        <p className="or">
          Or:{' '}
          <button type="button" className="link-button" onClick={() => onChoose('own_key')}>
            Use my own key
          </button>{' '}
          ·{' '}
          <button type="button" className="link-button" onClick={() => onChoose('local')}>
            Use a local model
          </button>
        </p>
      )}
    </section>
  )
}
