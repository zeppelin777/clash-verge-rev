import { DeleteOutlineRounded, EditRounded } from '@mui/icons-material'
import {
  Button,
  FormControlLabel,
  IconButton,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { useLockFn } from 'ahooks'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog, Switch } from '@/components/base'
import { useOnDemandStatus } from '@/hooks/use-on-demand-status'
import { useVerge } from '@/hooks/use-verge'
import type { OnDemandConfig_Serialize as OnDemandConfig } from '@/services/bindings'
import { showNotice } from '@/services/notice-service'

import { SettingItem } from './setting-comp'

const STATUS_KEYS = {
  disabled: 'settings.sections.onDemand.status.disabled',
  waiting: 'settings.sections.onDemand.status.waiting',
  permission_required: 'settings.sections.onDemand.status.permissionRequired',
  excluded: 'settings.sections.onDemand.status.excluded',
  allowed: 'settings.sections.onDemand.status.allowed',
} as const

const EMPTY_ENTRY = { original: null as string | null, ssid: '' }

export function OnDemandSetting() {
  const { t } = useTranslation()
  const { verge, patchVerge } = useVerge()
  const { data: status, error } = useOnDemandStatus()
  const [draft, setDraft] = useState<OnDemandConfig | null>(null)
  const [entry, setEntry] = useState(EMPTY_ENTRY)
  const [saving, setSaving] = useState(false)
  const saved = verge?.on_demand
  const statusKey = status?.apply_failed
    ? 'settings.sections.onDemand.status.applyFailed'
    : !saved?.enabled
      ? STATUS_KEYS.disabled
      : saved.excluded_ssids.length === 0
        ? 'settings.sections.onDemand.status.empty'
        : error
          ? 'settings.sections.onDemand.status.unavailable'
          : status
            ? STATUS_KEYS[status.state]
            : STATUS_KEYS.waiting

  const entryPending = entry.original !== null || entry.ssid.length > 0
  const entryError = !entryPending
    ? undefined
    : entry.ssid.length === 0 ||
        entry.ssid.includes('\0') ||
        new TextEncoder().encode(entry.ssid).length > 32
      ? 'settings.sections.onDemand.invalidSsid'
      : entry.original !== entry.ssid &&
          draft?.excluded_ssids.includes(entry.ssid)
        ? 'settings.sections.onDemand.duplicateSsid'
        : undefined

  const applyEntry = (ssids: string[]) =>
    entry.original !== null
      ? ssids.map((ssid) => (ssid === entry.original ? entry.ssid : ssid))
      : [...ssids, entry.ssid]

  const addOrUpdate = () => {
    if (!draft || !entryPending || entryError) return
    setDraft({ ...draft, excluded_ssids: applyEntry(draft.excluded_ssids) })
    setEntry(EMPTY_ENTRY)
  }

  const close = () => {
    if (!saving) setDraft(null)
  }

  const save = useLockFn(async () => {
    if (!draft || entryError) return
    setSaving(true)
    try {
      await patchVerge({
        on_demand: {
          ...draft,
          excluded_ssids: entryPending
            ? applyEntry(draft.excluded_ssids)
            : draft.excluded_ssids,
        },
      })
      setDraft(null)
    } catch (error) {
      showNotice.error(error)
    } finally {
      setSaving(false)
    }
  })

  return (
    <>
      <SettingItem
        data-setting-target="settings.sections.onDemand.title"
        label={t('settings.sections.onDemand.title')}
        secondary={t(statusKey)}
        onClick={
          verge
            ? () => {
                setEntry(EMPTY_ENTRY)
                setDraft({
                  enabled: saved?.enabled ?? false,
                  excluded_ssids: [...new Set(saved?.excluded_ssids)],
                })
              }
            : undefined
        }
      />

      {draft && (
        <BaseDialog
          open
          title={t('settings.sections.onDemand.title')}
          contentSx={{ width: 440, maxWidth: 'calc(100vw - 64px)' }}
          cancelBtn={t('shared.actions.cancel')}
          okBtn={t('shared.actions.save')}
          okDisabled={!!entryError}
          loading={saving}
          onOk={() => {
            void save()
          }}
          onClose={close}
          onCancel={close}
        >
          <Stack spacing={2}>
            <Stack spacing={0.75}>
              <FormControlLabel
                control={
                  <Switch
                    checked={draft.enabled}
                    disabled={saving}
                    onChange={(_, enabled) => setDraft({ ...draft, enabled })}
                    sx={{ ml: 1, mr: 1.5 }}
                  />
                }
                label={t('settings.sections.onDemand.enable')}
              />
              <Typography variant="body2" color="text.secondary">
                {t('settings.sections.onDemand.description')}
              </Typography>
              <Typography
                variant="body2"
                color={status?.apply_failed ? 'error.main' : 'text.secondary'}
                role="status"
              >
                {t('settings.sections.onDemand.status.current', {
                  status: t(statusKey),
                })}
              </Typography>
              {(status?.system_proxy_paused || status?.tun_paused) && (
                <Typography variant="body2" color="text.secondary">
                  {[
                    status.system_proxy_paused &&
                      t('settings.sections.onDemand.status.systemProxyPaused'),
                    status.tun_paused &&
                      t('settings.sections.onDemand.status.tunPaused'),
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </Typography>
              )}
            </Stack>

            <Stack spacing={1}>
              <Typography component="h3" variant="subtitle2">
                {t('settings.sections.onDemand.exclusions')}
              </Typography>
              {draft.excluded_ssids.map((ssid) => (
                <Stack
                  key={ssid}
                  direction="row"
                  sx={{ alignItems: 'center', gap: 1 }}
                >
                  <Typography
                    variant="body2"
                    sx={{
                      flex: 1,
                      whiteSpace: 'pre-wrap',
                      overflowWrap: 'anywhere',
                    }}
                  >
                    {ssid}
                  </Typography>
                  <IconButton
                    size="small"
                    aria-label={t('settings.sections.onDemand.editSsid', {
                      ssid,
                    })}
                    disabled={saving}
                    onClick={() => setEntry({ original: ssid, ssid })}
                  >
                    <EditRounded fontSize="small" />
                  </IconButton>
                  <IconButton
                    size="small"
                    aria-label={t('settings.sections.onDemand.removeSsid', {
                      ssid,
                    })}
                    disabled={saving}
                    onClick={() => {
                      setDraft({
                        ...draft,
                        excluded_ssids: draft.excluded_ssids.filter(
                          (item) => item !== ssid,
                        ),
                      })
                      if (entry.original === ssid) setEntry(EMPTY_ENTRY)
                    }}
                  >
                    <DeleteOutlineRounded fontSize="small" />
                  </IconButton>
                </Stack>
              ))}

              <Stack direction="row" sx={{ alignItems: 'flex-start', gap: 1 }}>
                <TextField
                  size="small"
                  fullWidth
                  label={t('settings.sections.onDemand.ssid')}
                  value={entry.ssid}
                  disabled={saving}
                  error={!!entryError}
                  helperText={entryError && t(entryError)}
                  onChange={(event) =>
                    setEntry({ ...entry, ssid: event.target.value })
                  }
                  onKeyDown={(event) => {
                    if (event.key !== 'Enter' || event.nativeEvent.isComposing)
                      return
                    event.preventDefault()
                    addOrUpdate()
                  }}
                />
                <Button
                  variant="outlined"
                  disabled={saving || !entryPending || !!entryError}
                  onClick={addOrUpdate}
                  sx={{ height: 40, whiteSpace: 'nowrap' }}
                >
                  {entry.original !== null
                    ? t('shared.actions.confirm')
                    : t('settings.sections.onDemand.addSsid')}
                </Button>
              </Stack>
              {entry.original !== null && (
                <Button
                  size="small"
                  onClick={() => setEntry(EMPTY_ENTRY)}
                  disabled={saving}
                  sx={{ alignSelf: 'flex-end' }}
                >
                  {t('shared.actions.cancel')}
                </Button>
              )}
            </Stack>
          </Stack>
        </BaseDialog>
      )}
    </>
  )
}
