import { Alert, AlertDescription, NewCard } from '@stump/components'
import { Info } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import { CreateOrUpdateLibrarySchema } from '@/components/library/createOrUpdate'
import { useTranslate } from '@/hooks/useTranslate'
import { useLibraryManagementSafe } from '@/scenes/library/tabs/settings/context'

type Props = {
	/**
	 * A callback that is triggered when the form values change, debounced by 1 second.
	 */
	onDidChange?: (
		values: Pick<
			CreateOrUpdateLibrarySchema,
			| 'processMetadata'
			| 'watch'
			| 'generateFileHashes'
			| 'generateKoreaderHashes'
			| 'useSeriesCoverImage'
			| 'cleanNames'
		>,
	) => void
}

export default function ScannerOptInFeatures({ onDidChange }: Props) {
	const form = useFormContext<CreateOrUpdateLibrarySchema>()
	const ctx = useLibraryManagementSafe()
	const isCreating = !ctx?.library

	const [
		processMetadata,
		watch,
		generateFileHashes,
		koreaderHashes,
		useSeriesCoverImage,
		cleanNames,
	] = useWatch({
		control: form.control,
		name: [
			'processMetadata',
			'watch',
			'generateFileHashes',
			'generateKoreaderHashes',
			'useSeriesCoverImage',
			'cleanNames',
		],
	})

	const params = useMemo(
		() => ({
			processMetadata,
			watch,
			generateFileHashes,
			generateKoreaderHashes: koreaderHashes,
			useSeriesCoverImage,
			cleanNames,
		}),
		[processMetadata, watch, generateFileHashes, koreaderHashes, useSeriesCoverImage, cleanNames],
	)

	const handleProcessMetadataChange = useCallback(() => {
		form.setValue('processMetadata', !processMetadata)
		if (onDidChange) {
			onDidChange({
				...params,
				processMetadata: !processMetadata,
			})
		}
	}, [form, processMetadata, params, onDidChange])

	const handleWatchChange = useCallback(() => {
		form.setValue('watch', !watch)
		if (onDidChange) {
			onDidChange({
				...params,
				watch: !watch,
			})
		}
	}, [form, watch, params, onDidChange])

	const handleGenerateFileHashesChange = useCallback(() => {
		form.setValue('generateFileHashes', !generateFileHashes)
		if (onDidChange) {
			onDidChange({
				...params,
				generateFileHashes: !generateFileHashes,
			})
		}
	}, [form, generateFileHashes, params, onDidChange])

	const handleGenerateKoreaderHashesChange = useCallback(() => {
		form.setValue('generateKoreaderHashes', !koreaderHashes)
		if (onDidChange) {
			onDidChange({
				...params,
				generateKoreaderHashes: !koreaderHashes,
			})
		}
	}, [form, koreaderHashes, params, onDidChange])

	const handleUseSeriesCoverImageChange = useCallback(() => {
		form.setValue('useSeriesCoverImage', !useSeriesCoverImage)
		if (onDidChange) {
			onDidChange({
				...params,
				useSeriesCoverImage: !useSeriesCoverImage,
			})
		}
	}, [form, useSeriesCoverImage, params, onDidChange])

	const handleCleanNamesChange = useCallback(() => {
		form.setValue('cleanNames', !cleanNames)
		if (onDidChange) {
			onDidChange({
				...params,
				cleanNames: !cleanNames,
			})
		}
	}, [form, cleanNames, params, onDidChange])

	const { t } = useTranslate()

	return (
		<div className="gap-y-6 flex flex-col">
			<NewCard label={t(getKey('section.heading'))} description={t(getKey('section.description'))}>
				<NewCard.CheckboxRow
					id="processMetadata"
					label={t(getKey('processMetadata.label'))}
					description={t(getKey('processMetadata.description'))}
					checked={processMetadata}
					onClick={handleProcessMetadataChange}
					{...form.register('processMetadata')}
				/>

				<NewCard.CheckboxRow
					id="watch"
					label={t(getKey('watch.label'))}
					description={t(getKey('watch.description'))}
					checked={watch}
					onClick={handleWatchChange}
					{...form.register('watch')}
				/>

				<NewCard.CheckboxRow
					id="generateFileHashes"
					label={t(getKey('generateFileHashes.label'))}
					description={t(getKey('generateFileHashes.description'))}
					checked={generateFileHashes}
					onClick={handleGenerateFileHashesChange}
					{...form.register('generateFileHashes')}
				/>

				<NewCard.CheckboxRow
					id="generateKoreaderHashes"
					label={t(getKey('koreaderHashes.label'))}
					description={t(getKey('koreaderHashes.description'))}
					checked={koreaderHashes}
					onClick={handleGenerateKoreaderHashesChange}
					{...form.register('generateKoreaderHashes')}
				/>

				<NewCard.CheckboxRow
					id="useSeriesCoverImage"
					label={t(getKey('useSeriesCoverImage.label'))}
					description={t(getKey('useSeriesCoverImage.description'))}
					checked={useSeriesCoverImage}
					onClick={handleUseSeriesCoverImageChange}
					{...form.register('useSeriesCoverImage')}
				/>

				<NewCard.CheckboxRow
					id="cleanNames"
					label={t(getKey('cleanNames.label'))}
					description={t(getKey('cleanNames.description'))}
					checked={cleanNames}
					onClick={handleCleanNamesChange}
					{...form.register('cleanNames')}
				/>
			</NewCard>

			{isCreating && (
				<Alert variant="info">
					<Info />
					<AlertDescription>{t(getKey('section.disclaimer'))}</AlertDescription>
				</Alert>
			)}
		</div>
	)
}

const LOCALE_KEY = 'createOrUpdateLibraryForm.fields.scannerFeatures'
const getKey = (key: string) => `${LOCALE_KEY}.${key}`
