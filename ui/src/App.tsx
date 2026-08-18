import { useEffect, useState } from '@lynx-js/react'

import './App.css'
import {
  getApplicationCardStates,
  getApplicationAccent,
  getApplicationInitial,
  hasApplicationIcon,
  type Application,
} from './applications.js'
import {
  getApplications,
  launchApplication,
} from './platform.js'

type LoadStatus = 'loading' | 'ready' | 'error'

interface InputEvent {
  detail: {
    value: string
  }
}

function errorMessage(prefix: string, error: unknown): string {
  if (error instanceof Error && error.message) {
    return `${prefix} ${error.message}`
  }

  return prefix
}

function ApplicationIcon({ application }: { application: Application }) {
  const [imageFailed, setImageFailed] = useState(false)
  const showImage = hasApplicationIcon(application) && !imageFailed

  if (showImage) {
    return (
      <view className='ApplicationIcon'>
        <image
          className='ApplicationImage'
          src={application.iconUri as string}
          mode='aspectFit'
          binderror={() => {
            'background only'
            setImageFailed(true)
          }}
        />
      </view>
    )
  }

  return (
    <view
      className={`ApplicationIcon ApplicationIcon--fallback ApplicationIcon--accent${getApplicationAccent(application)}`}
    >
      <text className='ApplicationInitial'>
        {getApplicationInitial(application.name)}
      </text>
    </view>
  )
}

export function App() {
  const [applications, setApplications] = useState<Application[]>([])
  const [loadStatus, setLoadStatus] = useState<LoadStatus>('loading')
  const [loadError, setLoadError] = useState('')
  const [query, setQuery] = useState('')
  const [launchingId, setLaunchingId] = useState<string | null>(null)
  const [launchError, setLaunchError] = useState('')

  async function loadApplicationList() {
    setLoadStatus('loading')
    setLoadError('')

    try {
      const nextApplications = await getApplications()
      setApplications(nextApplications)
      setLoadStatus('ready')
    } catch (error) {
      setLoadError(errorMessage('The platform could not provide applications.', error))
      setLoadStatus('error')
    }
  }

  useEffect(() => {
    void loadApplicationList()
  }, [])

  async function handleLaunch(id: string) {
    if (launchingId !== null) {
      return
    }

    setLaunchingId(id)
    setLaunchError('')

    try {
      await launchApplication(id)
    } catch (error) {
      setLaunchError(errorMessage('Launch failed.', error))
    } finally {
      setLaunchingId(null)
    }
  }

  const applicationCards = getApplicationCardStates(applications, query)
  const visibleApplicationCount = applicationCards.reduce(
    (count, card) => count + (card.visible ? 1 : 0),
    0,
  )
  const hasQuery = query.trim().length > 0
  const countLabel = loadStatus === 'ready'
    ? `${applications.length} ${applications.length === 1 ? 'application' : 'applications'}`
    : loadStatus === 'loading'
      ? 'Scanning applications'
      : 'Platform unavailable'

  return (
    <view className='Root'>
      <view className='Ambient Ambient--orange' />
      <view className='Ambient Ambient--mint' />

      <scroll-view className='Scroller' scroll-orientation='vertical'>
        <view className='Page'>
          <view className='Header'>
            <view className='Brand'>
              <view className='BrandMark'>
                <text className='BrandLetter'>L</text>
              </view>
              <view className='BrandCopy'>
                <text className='Eyebrow'>LYNX / DESKTOP</text>
                <text className='Title'>Launcher</text>
              </view>
            </view>

            <view className='CountBlock'>
              <view className={`StatusDot StatusDot--${loadStatus}`} />
              <text className='Count'>{countLabel}</text>
            </view>
          </view>

          <view className='SearchPanel'>
            <view className='SearchGlyph'>
              <view className='SearchGlyphRing' />
              <view className='SearchGlyphHandle' />
            </view>
            <input
              className='SearchInput'
              type='text'
              placeholder='Search installed applications'
              bindinput={(event: InputEvent) => {
                'background only'
                setQuery(event.detail.value)
              }}
            />
            <view className='SearchMeta'>
              <text className='SearchMetaText'>
                {loadStatus === 'ready'
                  ? `${visibleApplicationCount} shown`
                  : 'SYSTEM INDEX'}
              </text>
            </view>
          </view>

          {launchError
            ? (
              <view className='ActionError'>
                <view className='ActionErrorMark' />
                <text className='ActionErrorText'>{launchError}</text>
              </view>
            )
            : null}

          <view className='SectionHeader'>
            <text className='SectionTitle'>APPLICATIONS</text>
            <view className='SectionRule' />
            <text className='SectionIndex'>01</text>
          </view>

          <view className='Grid'>
            {applicationCards.map(card => {
              const { application, visiblePosition } = card
              const isVisible = loadStatus === 'ready' && card.visible
              const isLaunching = launchingId === application.id
              let cardClassName = 'ApplicationCard'
              if (isLaunching) {
                cardClassName += ' ApplicationCard--launching'
              }
              if (!isVisible) {
                cardClassName += ' ApplicationCard--hidden'
              }

              return (
                <view
                  className={cardClassName}
                  key={application.id}
                  bindtap={() => {
                    'background only'
                    if (!isVisible) {
                      return
                    }
                    void handleLaunch(application.id)
                  }}
                >
                  <ApplicationIcon application={application} />
                  <view className='ApplicationCopy'>
                    <text className='ApplicationNumber'>
                      {visiblePosition === null
                        ? '--'
                        : String(visiblePosition).padStart(2, '0')}
                    </text>
                    <text className='ApplicationName'>{application.name}</text>
                  </view>
                  <view className='LaunchBadge'>
                    <text className='LaunchBadgeText'>
                      {isLaunching ? 'WAIT' : 'OPEN'}
                    </text>
                  </view>
                </view>
              )
            })}
          </view>

          {loadStatus === 'error'
            ? (
              <view className='StatePanel StatePanel--error'>
                <view className='StateSymbol'>
                  <text className='StateSymbolText'>!</text>
                </view>
                <text className='StateKicker'>PLATFORM ERROR</text>
                <text className='StateTitle'>Applications are out of reach</text>
                <text className='StateBody'>{loadError}</text>
                <view
                  className='RetryButton'
                  bindtap={() => {
                    'background only'
                    void loadApplicationList()
                  }}
                >
                  <text className='RetryButtonText'>TRY AGAIN</text>
                </view>
              </view>
            )
            : null}

          {loadStatus === 'ready' && applications.length === 0
            ? (
              <view className='StatePanel'>
                <view className='StateSymbol StateSymbol--empty'>
                  <view className='EmptyGlyphLine EmptyGlyphLine--top' />
                  <view className='EmptyGlyphLine EmptyGlyphLine--bottom' />
                </view>
                <text className='StateKicker'>EMPTY LIBRARY</text>
                <text className='StateTitle'>No applications found</text>
                <text className='StateBody'>
                  The platform returned no launchable applications.
                </text>
              </view>
            )
            : null}

          {loadStatus === 'ready' && applications.length > 0
            && visibleApplicationCount === 0
            ? (
              <view className='StatePanel'>
                <view className='StateSymbol StateSymbol--empty'>
                  <view className='SearchGlyphRing StateSearchRing' />
                  <view className='SearchGlyphHandle StateSearchHandle' />
                </view>
                <text className='StateKicker'>NO MATCHES</text>
                <text className='StateTitle'>Nothing answers that search</text>
                <text className='StateBody'>
                  Try a shorter name or a different spelling.
                </text>
              </view>
            )
            : null}

          <view className='Footer'>
            <text className='FooterText'>SELECT AN APPLICATION TO LAUNCH</text>
            <text className='FooterText FooterText--right'>
              {hasQuery ? 'FILTER ACTIVE' : 'READY'}
            </text>
          </view>
        </view>
      </scroll-view>
    </view>
  )
}
