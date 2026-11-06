import { getOnDemandStatus } from '@/services/cmds'
import { useQuery } from '@/services/query-client'
import getSystem from '@/utils/get-system'

export const useOnDemandStatus = () =>
  useQuery({
    queryKey: ['getOnDemandStatus'],
    queryFn: getOnDemandStatus,
    enabled: getSystem() === 'macos',
  })
