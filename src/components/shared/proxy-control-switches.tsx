import {
  DeleteForeverRounded,
  PauseCircleOutlineRounded,
  PlayCircleOutlineRounded,
  SettingsRounded,
  WarningRounded,
} from '@mui/icons-material'
import { Box, Typography, alpha, useTheme } from '@mui/material'
import { useLockFn } from 'ahooks'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { type DialogRef, Switch, TooltipIcon } from '@/components/base'
import { SysproxyViewer } from '@/components/setting/mods/sysproxy-viewer'
import { TunViewer } from '@/components/setting/mods/tun-viewer'
import { useOnDemandStatus } from '@/hooks/use-on-demand-status'
import { useServiceUninstaller } from '@/hooks/use-service-uninstaller'
import { useSystemProxyState } from '@/hooks/use-system-proxy-state'
import { useSystemState } from '@/hooks/use-system-state'
import { useVerge } from '@/hooks/use-verge'
import { showNotice } from '@/services/notice-service'
import { requestService } from '@/services/service-request'

interface ProxySwitchProps {
  label?: string
  onError?: (err: Error) => void
  noRightPadding?: boolean
}

interface SwitchRowProps {
  label: string
  active: boolean
  preferred?: boolean
  secondary?: string
  disabled?: boolean
  infoTitle: string
  onInfoClick?: () => void
  extraIcons?: React.ReactNode
  /** Return false to roll back without reporting an error. */
  onToggle: (value: boolean) => Promise<boolean | void>
  onError?: (err: Error) => void
  highlight?: boolean
}

const SwitchRow = ({
  label,
  active,
  preferred = active,
  secondary,
  disabled,
  infoTitle,
  onInfoClick,
  extraIcons,
  onToggle,
  onError,
  highlight,
}: SwitchRowProps) => {
  const theme = useTheme()
  const [checked, setChecked] = useState(preferred)
  const pendingRef = useRef(false)

  if (pendingRef.current) {
    if (preferred === checked) pendingRef.current = false
  } else if (checked !== preferred) {
    setChecked(preferred)
  }

  const handleChange = (_: React.ChangeEvent, value: boolean) => {
    pendingRef.current = true
    setChecked(value)
    onToggle(value)
      .then((applied) => {
        if (applied === false) setChecked(preferred)
      })
      .catch((err: any) => {
        setChecked(preferred)
        onError?.(err)
      })
      .finally(() => {
        pendingRef.current = false
      })
  }

  return (
    <Box
      sx={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        p: 1,
        pr: 2,
        borderRadius: 1.5,
        bgcolor: highlight
          ? alpha(theme.palette.success.main, 0.07)
          : 'transparent',
        opacity: disabled ? 0.6 : 1,
        transition: 'background-color 0.3s',
      }}
    >
      <Box sx={{ display: 'flex', alignItems: 'center' }}>
        {active ? (
          <PlayCircleOutlineRounded sx={{ color: 'success.main', mr: 1 }} />
        ) : (
          <PauseCircleOutlineRounded sx={{ color: 'text.disabled', mr: 1 }} />
        )}
        <Box>
          <Typography
            variant="subtitle1"
            sx={{ fontWeight: 500, fontSize: '15px' }}
          >
            {label}
          </Typography>
          {secondary && (
            <Typography variant="caption" color="text.secondary">
              {secondary}
            </Typography>
          )}
        </Box>
        <TooltipIcon
          title={infoTitle}
          icon={SettingsRounded}
          onClick={onInfoClick}
          sx={{ ml: 1 }}
        />
        {extraIcons}
      </Box>

      <Switch
        edge="end"
        disabled={disabled}
        checked={checked}
        onChange={handleChange}
        slotProps={{ input: { 'aria-label': label } }}
      />
    </Box>
  )
}

const ProxyControlSwitches = ({
  label,
  onError,
  noRightPadding = false,
}: ProxySwitchProps) => {
  const { t } = useTranslation()
  const { verge, mutateVerge, patchVerge } = useVerge()
  const { uninstallServiceAndStartSidecar } = useServiceUninstaller()
  const { indicator: systemProxyIndicator, toggleSystemProxy } =
    useSystemProxyState()
  const { data: onDemand } = useOnDemandStatus()
  const { runState, isTunModeAvailable, isLoading } = useSystemState()
  // Offer to uninstall only a service that is actually there and working.
  const isServiceInstallReady = runState.serviceUsable

  const sysproxyRef = useRef<DialogRef>(null)
  const tunRef = useRef<DialogRef>(null)

  const { enable_system_proxy, enable_tun_mode } = verge ?? {}
  const managedOnDemand = onDemand && onDemand.state !== 'disabled'
  const systemProxyPaused =
    !!enable_system_proxy && !!onDemand?.system_proxy_paused
  const tunPaused = !!enable_tun_mode && !!onDemand?.tun_paused
  const systemProxyActive = systemProxyIndicator && !systemProxyPaused
  const tunActive =
    managedOnDemand || onDemand?.apply_failed
      ? !!onDemand?.tun_active
      : !!enable_tun_mode && isTunModeAvailable
  const pausedLabel = t('settings.sections.onDemand.status.paused')

  // Enabling needs a running core; disabling only writes OS state and must stay available.
  const handleSystemProxyToggle = async (value: boolean) => {
    if (value && !isLoading && runState.mode === 'NotRunning') {
      showNotice.error('settings.feedback.errors.sysproxy.coreNotReady')
      return false
    }
    await toggleSystemProxy(value)
  }

  const handleTunToggle = async (value: boolean) => {
    if (value && !isTunModeAvailable) {
      requestService({
        reason: 'tunNeedsService',
        restore: { enable_tun_mode: value },
      })
      return false
    }
    mutateVerge({ ...verge, enable_tun_mode: value })
    await patchVerge({ enable_tun_mode: value })
  }

  const onUninstallService = useLockFn(async () => {
    try {
      await uninstallServiceAndStartSidecar()
    } catch (err) {
      showNotice.error(err)
    }
  })

  const isSystemProxyMode =
    label === t('settings.sections.system.toggles.systemProxy') || !label
  const isTunMode = label === t('settings.sections.system.toggles.tunMode')

  return (
    <Box sx={{ width: '100%', pr: noRightPadding ? 1 : 2 }}>
      {isSystemProxyMode && (
        <SwitchRow
          label={t('settings.sections.proxyControl.fields.systemProxy')}
          active={systemProxyActive}
          preferred={
            managedOnDemand || systemProxyPaused
              ? !!enable_system_proxy
              : undefined
          }
          secondary={systemProxyPaused ? pausedLabel : undefined}
          infoTitle={t('settings.sections.proxyControl.tooltips.systemProxy')}
          onInfoClick={() => sysproxyRef.current?.open()}
          onToggle={handleSystemProxyToggle}
          onError={onError}
          highlight={systemProxyActive}
        />
      )}

      {isTunMode && (
        <SwitchRow
          label={t('settings.sections.proxyControl.fields.tunMode')}
          active={tunActive}
          preferred={
            managedOnDemand || tunPaused ? !!enable_tun_mode : undefined
          }
          secondary={tunPaused ? pausedLabel : undefined}
          infoTitle={t('settings.sections.proxyControl.tooltips.tunMode')}
          onInfoClick={() => tunRef.current?.open()}
          onToggle={handleTunToggle}
          onError={onError}
          highlight={tunActive}
          extraIcons={
            <>
              {!isTunModeAvailable && (
                <TooltipIcon
                  title={t(
                    'settings.sections.proxyControl.tooltips.tunUnavailable',
                  )}
                  icon={WarningRounded}
                  sx={{ color: 'warning.main', ml: 1 }}
                />
              )}
              {isServiceInstallReady && (
                <TooltipIcon
                  title={t(
                    'settings.sections.proxyControl.actions.uninstallService',
                  )}
                  icon={DeleteForeverRounded}
                  color="secondary"
                  data-setting-target="service-uninstall"
                  onClick={onUninstallService}
                  sx={{ ml: 1 }}
                />
              )}
            </>
          }
        />
      )}

      {isSystemProxyMode && <SysproxyViewer ref={sysproxyRef} />}
      {isTunMode && <TunViewer ref={tunRef} />}
    </Box>
  )
}

export default ProxyControlSwitches
