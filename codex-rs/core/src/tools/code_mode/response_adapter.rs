use codex_code_mode::ImageDetail as CodeModeImageDetail;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ImageDetail;
use codex_protocol::models::validate_prompt_image_url;

trait IntoProtocol<T> {
    fn into_protocol(self) -> T;
}

pub(super) fn into_function_call_output_content_items(
    items: Vec<codex_code_mode::FunctionCallOutputContentItem>,
) -> Vec<FunctionCallOutputContentItem> {
    items.into_iter().map(IntoProtocol::into_protocol).collect()
}

impl IntoProtocol<ImageDetail> for CodeModeImageDetail {
    fn into_protocol(self) -> ImageDetail {
        let value = self;
        match value {
            CodeModeImageDetail::High => ImageDetail::High,
            CodeModeImageDetail::Original => ImageDetail::Original,
        }
    }
}

impl IntoProtocol<FunctionCallOutputContentItem>
    for codex_code_mode::FunctionCallOutputContentItem
{
    fn into_protocol(self) -> FunctionCallOutputContentItem {
        let value = self;
        match value {
            codex_code_mode::FunctionCallOutputContentItem::InputText { text } => {
                FunctionCallOutputContentItem::InputText { text }
            }
            codex_code_mode::FunctionCallOutputContentItem::InputImage { image_url, detail } => {
                if validate_prompt_image_url(&image_url).is_ok() {
                    FunctionCallOutputContentItem::InputImage {
                        image_url,
                        detail: detail
                            .map(IntoProtocol::into_protocol)
                            .or(Some(DEFAULT_IMAGE_DETAIL)),
                    }
                } else {
                    FunctionCallOutputContentItem::InputText {
                        text: "Invalid image".to_string(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn invalid_inline_image_outputs_become_text() {
        let items = into_function_call_output_content_items(vec![
            codex_code_mode::FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,aGVsbG8=".to_string(),
                detail: None,
            },
        ]);

        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputText {
                text: "Invalid image".to_string(),
            }]
        );
    }
}
