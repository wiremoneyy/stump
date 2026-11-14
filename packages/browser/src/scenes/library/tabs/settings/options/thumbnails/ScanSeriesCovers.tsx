import { useGraphQLMutation } from '@stump/client'
import { Button, NewCard } from '@stump/components'
import { graphql } from '@stump/graphql'

import { useLibraryContext } from '@/scenes/library/context'

const mutation = graphql(`
	mutation ScanLibrarySeriesCovers($id: ID!) {
		scanLibrarySeriesCovers(id: $id)
	}
`)

export default function ScanSeriesCovers() {
	const { library } = useLibraryContext()
	const { mutate, isPending } = useGraphQLMutation(mutation)

	return (
		<NewCard.Row
			label="Scan series covers"
			description="Apply cover images (cover, folder or poster) found in series folders. This runs separately from library scans and only touches series thumbnails."
		>
			<div>
				<Button
					variant="outline"
					disabled={isPending}
					onClick={() => mutate({ id: library.id })}
				>
					Scan covers
				</Button>
			</div>
		</NewCard.Row>
	)
}
