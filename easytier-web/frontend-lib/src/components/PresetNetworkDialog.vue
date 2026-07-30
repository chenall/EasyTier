<script setup lang="ts">
import { Button, ConfirmDialog, Dialog, InputText, useConfirm, useToast } from 'primevue';
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import * as Api from '../modules/api';
import * as NetworkTypes from '../types/network';
import { DEFAULT_NETWORK_CONFIG, normalizeNetworkConfig } from '../types/network';
import Config from './Config.vue';
import ConfigEditDialog from './ConfigEditDialog.vue';

const props = defineProps<{
  api: Api.PresetClient;
}>();

const emit = defineEmits<{
  (e: 'view-group', preset: NetworkTypes.PresetSummary): void;
}>();

const { t } = useI18n();
const toast = useToast();
const confirm = useConfirm();

const presets = ref<Array<NetworkTypes.PresetSummary>>([]);
const loading = ref(false);

const dialogVisible = ref(false);
const editingId = ref<number | null>(null); // null => create mode
const editingName = ref('');
const editingConfig = ref<NetworkTypes.NetworkConfig>(DEFAULT_NETWORK_CONFIG());
const saving = ref(false);
const showConfigEditDialog = ref(false);

// "Edit as file" — serialize the template NetworkConfig to a raw config text and
// back. Mirrors the device-side flow in RemoteManagement.vue, but presets have no
// instance_id, so we assign the parsed config straight to the editing target.
const generateConfig = async (config: NetworkTypes.NetworkConfig): Promise<string> => {
  const { toml_config: tomlConfig, error } = await props.api.generate_config(config);
  if (error) {
    throw error;
  }
  return tomlConfig ?? '';
};

const syncTomlConfig = async (tomlConfig: string): Promise<void> => {
  const resp = await props.api.parse_config(tomlConfig);
  if (resp.error) {
    throw resp.error;
  }
  const config = resp.config;
  if (!config) {
    throw new Error('Parsed config is empty');
  }
  editingConfig.value = config;
};

const loadPresets = async () => {
  loading.value = true;
  try {
    presets.value = await props.api.list_presets();
  } catch (e: any) {
    toast.add({ severity: 'error', summary: 'Error', detail: String(e?.response?.data ?? e), life: 2000 });
  } finally {
    loading.value = false;
  }
};

onMounted(loadPresets);

const openCreate = () => {
  editingId.value = null;
  editingName.value = '';
  editingConfig.value = DEFAULT_NETWORK_CONFIG();
  dialogVisible.value = true;
};

const openEdit = (p: NetworkTypes.PresetSummary) => {
  editingId.value = p.id;
  editingName.value = p.name;
  editingConfig.value = normalizeNetworkConfig(p.network_config);
  dialogVisible.value = true;
};

const save = async () => {
  if (!editingName.value.trim()) {
    toast.add({
      severity: 'warn',
      summary: t('web.common.warning'),
      detail: t('web.preset.name_required'),
      life: 2000,
    });
    return;
  }
  saving.value = true;
  try {
    if (editingId.value === null) {
      await props.api.create_preset(editingName.value.trim(), editingConfig.value);
    } else {
      await props.api.update_preset(editingId.value, editingName.value.trim(), editingConfig.value);
    }
    dialogVisible.value = false;
    await loadPresets();
    toast.add({
      severity: 'success',
      summary: t('web.common.success'),
      detail: t('web.preset.saved'),
      life: 2000,
    });
  } catch (e: any) {
    toast.add({
      severity: 'error',
      summary: 'Error',
      detail: JSON.stringify(e?.response?.data ?? e),
      life: 3000,
    });
  } finally {
    saving.value = false;
  }
};

const confirmDelete = (p: NetworkTypes.PresetSummary, event: Event) => {
  confirm.require({
    target: event.currentTarget as any,
    message: t('web.preset.delete_confirm', { name: p.name }),
    icon: 'pi pi-info-circle',
    rejectProps: { label: t('web.common.cancel'), severity: 'secondary', outlined: true },
    acceptProps: { label: t('web.common.delete'), severity: 'danger' },
    accept: async () => {
      try {
        await props.api.delete_preset(p.id);
        await loadPresets();
      } catch (e: any) {
        toast.add({
          severity: 'error',
          summary: 'Error',
          detail: JSON.stringify(e?.response?.data ?? e),
          life: 3000,
        });
      }
    },
  });
};
</script>

<template>
  <div class="flex flex-col gap-4">
    <div class="flex items-center justify-between">
      <h3 class="text-lg font-semibold">{{ t('web.preset.title') }}</h3>
      <Button :label="t('web.preset.create')" icon="pi pi-plus" @click="openCreate" />
    </div>

    <div v-if="loading" class="text-secondary">{{ t('web.common.loading') }}</div>
    <div v-else-if="presets.length === 0" class="text-secondary">{{ t('web.preset.list_empty') }}</div>

    <div v-else class="flex flex-col gap-2">
      <div
        v-for="p in presets"
        :key="p.id"
        class="flex items-center justify-between border rounded-md p-3 surface-0"
      >
        <div class="min-w-0">
          <div class="font-medium truncate">{{ p.name }}</div>
          <div class="text-sm text-secondary truncate">
            {{ p.network_config.network_name || t('web.preset.unknown_network') }}
          </div>
        </div>
        <div class="flex gap-2 shrink-0">
          <Button
            icon="pi pi-sitemap"
            severity="info"
            text
            rounded
            :title="t('web.preset.view_group')"
            @click="emit('view-group', p)"
          />
          <Button
            icon="pi pi-pencil"
            severity="secondary"
            text
            rounded
            :title="t('web.common.edit')"
            @click="openEdit(p)"
          />
          <Button
            icon="pi pi-trash"
            severity="danger"
            text
            rounded
            :title="t('web.common.delete')"
            @click="confirmDelete(p, $event)"
          />
        </div>
      </div>
    </div>

    <Dialog
      v-model:visible="dialogVisible"
      modal
      :header="editingId === null ? t('web.preset.create') : t('web.preset.edit')"
      :style="{ width: '90vw', maxWidth: '720px' }"
    >
      <div class="flex flex-col gap-4">
        <div class="flex flex-col gap-1">
          <label class="text-sm font-medium">{{ t('web.preset.name') }}</label>
          <InputText
            v-model="editingName"
            :placeholder="t('web.preset.name_placeholder')"
            class="w-full"
          />
        </div>
        <div class="flex flex-col gap-1">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium">{{ t('web.preset.config') }}</label>
            <Button type="button" @click="showConfigEditDialog = true" icon="pi pi-file-edit"
              :label="t('web.device_management.edit_as_file')" iconPos="left" severity="secondary" />
          </div>
          <Config v-model:cur-network="editingConfig" />
        </div>
      </div>

      <ConfigEditDialog v-model:visible="showConfigEditDialog" :cur-network="editingConfig"
        :generate-config="generateConfig" :save-config="syncTomlConfig" />
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button
            :label="t('web.common.cancel')"
            severity="secondary"
            text
            @click="dialogVisible = false"
          />
          <Button :label="t('web.common.save')" :loading="saving" @click="save" />
        </div>
      </template>
    </Dialog>

    <ConfirmDialog></ConfirmDialog>
  </div>
</template>
