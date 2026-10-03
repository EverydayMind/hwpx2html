//! All fixed emitter/caption reasons and their Korean presentation.
//! Keep English detail unchanged: CLI warnings and errors use it verbatim.

pub struct Reason {
    pub code: &'static str,
    pub detail: &'static str,
    pub message: &'static str,
    pub action: &'static str,
}

pub const CAPTION_ON_A_TABLE_AFTER_ANOTHER_IN_ITS_LINE: &str =
    "a caption on a table after another in its line";
pub const INLINE_TABLE_IN_NO_LINE: &str = "an inline table in no line";
pub const CAPTION_ON_A_FLOATING_TABLE_WITH_OUTER_MARGINS: &str =
    "a caption on a floating table with outer margins";
pub const EQUATION_WITHOUT_ITS_SCRIPT: &str = "an equation without its script";
pub const EQUATION_NOT_SET_IN_A_LINE: &str = "an equation not set in a line";
pub const PICTURE_WITHOUT_A_SUPPORTED_IMAGE_RESOURCE: &str =
    "a picture without a supported image resource";
pub const UNSUPPORTED_OBJECT: &str = "an unsupported object";
pub const CAPTION_WITH_TEXT: &str = "a caption with text";
pub const TEXT_BOX_TABLE_NOT_LAID_OUT: &str = "a text box table not laid out";
pub const NESTED_TABLE_DRAWN_MORE_THAN_ONCE: &str = "a nested table drawn more than once";
pub const FLOATING_OBJECT_IN_A_REPEATED_HEADER_ROW: &str =
    "a floating object in a repeated header row";
pub const TABLE_IN_A_REPEATED_HEADER_ROW: &str = "a table in a repeated header row";
pub const OBJECT_WITH_TEXT_IN_A_REPEATED_HEADER_ROW: &str =
    "an object with text in a repeated header row";
pub const CELL_OUTSIDE_ITS_TABLE: &str = "a cell outside its table";
pub const CAPTION_OUTSIDE_ITS_TABLE: &str = "a caption outside its table";
pub const GENERATED_NUMBER_WITHOUT_ITS_CONTROL: &str = "a generated number without its control";
pub const NUMBER_IN_AN_UNKNOWN_FORMAT: &str = "a number in an unknown format";
pub const ANNOTATION_WITHOUT_ITS_CONTROL: &str = "an annotation without its control";
pub const PARAGRAPH_THE_LAYOUT_DREW_NO_LINE_FOR: &str = "a paragraph the layout drew no line for";
pub const HEADING_CARRYING_A_TABLE: &str = "a heading carrying a table";
pub const PARAGRAPH_CHILD: &str = "a paragraph child";
pub const ANNOTATION_IN_A_DRAWN_ONLY_COPY: &str = "an annotation in a drawn-only copy";
pub const ANNOTATION_POSITION_THE_CORPUS_DOES_NOT_SHOW: &str =
    "an annotation position the corpus does not show";
pub const ANNOTATION_SIZE_ALIGNMENT_OR_OPTION_NOT_SHOWN: &str =
    "an annotation size, alignment or option not shown";
pub const ANNOTATION_WITHOUT_ITS_LETTERS_OR_ITS_LINE: &str =
    "an annotation without its letters or its line";
pub const OVERLAPPED_LETTERS_SHAPE_THE_CORPUS_DOES_NOT_SHOW: &str =
    "an overlapped-letters shape the corpus does not show";
pub const OVERLAPPED_LETTERS_THE_CORPUS_DOES_NOT_SHOW: &str =
    "overlapped letters the corpus does not show";
pub const OVERLAPPED_LETTERS_IN_LETTERS_OF_THEIR_OWN: &str =
    "overlapped letters in letters of their own";
pub const ANNOTATION_OF_NO_KNOWN_KIND: &str = "an annotation of no known kind";
pub const BLOCK_CONTENT_IN_A_PHRASING_OBJECT: &str = "block content in a phrasing object";
pub const TEXT_BOX_IN_A_DRAWN_ONLY_COPY: &str = "a text box in a drawn-only copy";
pub const EQUATION_IN_A_DRAWN_ONLY_COPY: &str = "an equation in a drawn-only copy";
pub const EQUATION_NOT_IN_THE_TREE: &str = "an equation not in the tree";
pub const TEXT_BOX_TABLE_NOT_IN_THE_TREE: &str = "a text box table not in the tree";
pub const TEXT_BOX_NOT_IN_THE_TREE: &str = "a text box not in the tree";
pub const OBJECT_THE_LAYOUT_DID_NOT_PLACE: &str = "an object the layout did not place";
pub const INLINE_TABLE_NOT_IN_THE_TREE: &str = "an inline table not in the tree";
pub const TABLE_NOT_IN_THE_SOURCE: &str = "a table not in the source";
pub const TABLE_THE_LAYOUT_DID_NOT_PLACE: &str = "a table the layout did not place";
pub const TABLE_CHILD: &str = "a table child";
pub const CAPTION_ON_A_TABLE_WITHOUT_A_GRID: &str = "a caption on a table without a grid";
pub const CELL_NOT_IN_THE_TREE: &str = "a cell not in the tree";
pub const CAPTION_CHILD: &str = "a caption child";
pub const CAPTION_ON_A_TABLE_SPLIT_ACROSS_PAGES: &str = "a caption on a table split across pages";
pub const LINE_OF_NO_PARAGRAPH_IN_THE_TREE: &str = "a line of no paragraph in the tree";
pub const TABLE_DRAWING_OF_NO_TABLE_IN_THE_TREE: &str = "a table drawing of no table in the tree";
pub const LINE_WRITTEN_TWICE: &str = "a line written twice";
pub const TABLE_OF_THE_TREE_NOT_WRITTEN: &str = "a table of the tree not written";
pub const CAPTION_BESIDE_OR_BELOW_ITS_TABLE: &str = "a caption beside or below its table";
pub const CAPTION_SPANNING_ITS_TABLE_S_MARGINS: &str = "a caption spanning its table's margins";
pub const CAPTION_WITHOUT_ITS_WIDTH: &str = "a caption without its width";
pub const CAPTION_HOLDING_A_TABLE_OR_OBJECT: &str = "a caption holding a table or object";
pub const CAPTION_PARAGRAPH_WITHOUT_ITS_LINE_POSITIONS: &str =
    "a caption paragraph without its line positions";
pub const CAPTION_WHOSE_LINES_RESTART: &str = "a caption whose lines restart";

pub const ALL: &[Reason] = &[
    Reason { code: "caption_on_a_table_after_another_in_its_line", detail: CAPTION_ON_A_TABLE_AFTER_ANOTHER_IN_ITS_LINE, message: "한 줄 안의 두 번째 표에 달린 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "inline_table_in_no_line", detail: INLINE_TABLE_IN_NO_LINE, message: "줄 위치가 없는 인라인 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "caption_on_a_floating_table_with_outer_margins", detail: CAPTION_ON_A_FLOATING_TABLE_WITH_OUTER_MARGINS, message: "바깥 여백이 있는 부동 표의 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "equation_without_its_script", detail: EQUATION_WITHOUT_ITS_SCRIPT, message: "원본 수식 스크립트가 없어 수식을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "equation_not_set_in_a_line", detail: EQUATION_NOT_SET_IN_A_LINE, message: "줄 안에 배치되지 않은 수식을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "picture_without_a_supported_image_resource", detail: PICTURE_WITHOUT_A_SUPPORTED_IMAGE_RESOURCE, message: "지원하는 이미지 자원이 없는 그림을 표시하지 못했습니다.", action: "한글에서 그림을 PNG나 JPEG로 바꿔 다시 저장해 보세요." },
    Reason { code: "unsupported_object", detail: UNSUPPORTED_OBJECT, message: "지원하지 않는 개체를 표시하지 못했습니다.", action: "한글에서 해당 개체를 PNG나 JPEG 그림으로 바꿔 다시 저장해 보세요." },
    Reason { code: "caption_with_text", detail: CAPTION_WITH_TEXT, message: "이 개체의 글자가 있는 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "text_box_table_not_laid_out", detail: TEXT_BOX_TABLE_NOT_LAID_OUT, message: "배치되지 않은 글상자 안의 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "nested_table_drawn_more_than_once", detail: NESTED_TABLE_DRAWN_MORE_THAN_ONCE, message: "여러 번 그려지는 중첩 표의 일부를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "floating_object_in_a_repeated_header_row", detail: FLOATING_OBJECT_IN_A_REPEATED_HEADER_ROW, message: "반복 머리행 안의 부동 개체를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_in_a_repeated_header_row", detail: TABLE_IN_A_REPEATED_HEADER_ROW, message: "반복 머리행 안의 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "object_with_text_in_a_repeated_header_row", detail: OBJECT_WITH_TEXT_IN_A_REPEATED_HEADER_ROW, message: "반복 머리행 안의 글자가 있는 개체를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "cell_outside_its_table", detail: CELL_OUTSIDE_ITS_TABLE, message: "표 밖에 있는 셀을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "caption_outside_its_table", detail: CAPTION_OUTSIDE_ITS_TABLE, message: "표 밖에 있는 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "generated_number_without_its_control", detail: GENERATED_NUMBER_WITHOUT_ITS_CONTROL, message: "번호 제어 정보가 없어 자동 번호를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "number_in_an_unknown_format", detail: NUMBER_IN_AN_UNKNOWN_FORMAT, message: "알 수 없는 번호 형식을 단순하게 표시했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "annotation_without_its_control", detail: ANNOTATION_WITHOUT_ITS_CONTROL, message: "제어 정보가 없는 덧말 또는 겹친 글자를 표시하지 못했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "paragraph_the_layout_drew_no_line_for", detail: PARAGRAPH_THE_LAYOUT_DREW_NO_LINE_FOR, message: "배치된 줄이 없는 문단을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "heading_carrying_a_table", detail: HEADING_CARRYING_A_TABLE, message: "표를 포함한 제목 문단의 일부를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "paragraph_child", detail: PARAGRAPH_CHILD, message: "문단 안의 일부 내용을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "annotation_in_a_drawn_only_copy", detail: ANNOTATION_IN_A_DRAWN_ONLY_COPY, message: "반복 표시 사본의 덧말 또는 겹친 글자를 표시하지 못했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "annotation_position_the_corpus_does_not_show", detail: ANNOTATION_POSITION_THE_CORPUS_DOES_NOT_SHOW, message: "지원하지 않는 위치의 덧말을 본문 글자만으로 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "annotation_size_alignment_or_option_not_shown", detail: ANNOTATION_SIZE_ALIGNMENT_OR_OPTION_NOT_SHOWN, message: "지원하지 않는 크기·정렬·옵션의 덧말을 본문 글자만으로 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "annotation_without_its_letters_or_its_line", detail: ANNOTATION_WITHOUT_ITS_LETTERS_OR_ITS_LINE, message: "글자나 줄 정보가 없는 덧말을 단순하게 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "overlapped_letters_shape_the_corpus_does_not_show", detail: OVERLAPPED_LETTERS_SHAPE_THE_CORPUS_DOES_NOT_SHOW, message: "지원하지 않는 모양의 겹친 글자를 단순하게 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "overlapped_letters_the_corpus_does_not_show", detail: OVERLAPPED_LETTERS_THE_CORPUS_DOES_NOT_SHOW, message: "지원 범위 밖의 겹친 글자를 단순하게 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "overlapped_letters_in_letters_of_their_own", detail: OVERLAPPED_LETTERS_IN_LETTERS_OF_THEIR_OWN, message: "독립 글자를 가진 겹친 글자를 단순하게 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "annotation_of_no_known_kind", detail: ANNOTATION_OF_NO_KNOWN_KIND, message: "알 수 없는 종류의 글자 주석을 단순하게 표시했습니다.", action: "원본의 덧말·겹친 글자와 대조해 주세요. 필요하면 일반 글자로 바꿔 다시 저장해 보세요." },
    Reason { code: "block_content_in_a_phrasing_object", detail: BLOCK_CONTENT_IN_A_PHRASING_OBJECT, message: "줄 안 개체가 가진 문단 또는 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "text_box_in_a_drawn_only_copy", detail: TEXT_BOX_IN_A_DRAWN_ONLY_COPY, message: "반복 표시 사본의 글상자를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "equation_in_a_drawn_only_copy", detail: EQUATION_IN_A_DRAWN_ONLY_COPY, message: "반복 표시 사본의 수식을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "equation_not_in_the_tree", detail: EQUATION_NOT_IN_THE_TREE, message: "문서 구조에 없는 수식을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "text_box_table_not_in_the_tree", detail: TEXT_BOX_TABLE_NOT_IN_THE_TREE, message: "문서 구조에 없는 글상자 안의 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "text_box_not_in_the_tree", detail: TEXT_BOX_NOT_IN_THE_TREE, message: "문서 구조에 없는 글상자를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "object_the_layout_did_not_place", detail: OBJECT_THE_LAYOUT_DID_NOT_PLACE, message: "배치되지 않은 개체를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "inline_table_not_in_the_tree", detail: INLINE_TABLE_NOT_IN_THE_TREE, message: "문서 구조에 없는 인라인 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_not_in_the_source", detail: TABLE_NOT_IN_THE_SOURCE, message: "원본에서 찾을 수 없는 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_the_layout_did_not_place", detail: TABLE_THE_LAYOUT_DID_NOT_PLACE, message: "배치되지 않은 표를 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_child", detail: TABLE_CHILD, message: "표 안의 일부 내용을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "caption_on_a_table_without_a_grid", detail: CAPTION_ON_A_TABLE_WITHOUT_A_GRID, message: "격자가 없는 표의 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "cell_not_in_the_tree", detail: CELL_NOT_IN_THE_TREE, message: "문서 구조에 없는 셀을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "caption_child", detail: CAPTION_CHILD, message: "캡션 안의 일부 내용을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_on_a_table_split_across_pages", detail: CAPTION_ON_A_TABLE_SPLIT_ACROSS_PAGES, message: "여러 쪽에 걸친 표의 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "line_of_no_paragraph_in_the_tree", detail: LINE_OF_NO_PARAGRAPH_IN_THE_TREE, message: "문서 구조에 대응하는 문단이 없는 줄을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_drawing_of_no_table_in_the_tree", detail: TABLE_DRAWING_OF_NO_TABLE_IN_THE_TREE, message: "문서 구조에 대응하는 표가 없는 표 그림을 표시하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "line_written_twice", detail: LINE_WRITTEN_TWICE, message: "같은 줄이 두 번 출력되어 변환을 중단했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "table_of_the_tree_not_written", detail: TABLE_OF_THE_TREE_NOT_WRITTEN, message: "문서 구조의 표를 출력하지 못했습니다.", action: "원본과 해당 위치의 미리보기를 대조해 주세요. 내용이 필요하면 한글에서 다시 저장하거나 지원하는 형태로 바꿔 주세요." },
    Reason { code: "caption_beside_or_below_its_table", detail: CAPTION_BESIDE_OR_BELOW_ITS_TABLE, message: "표 옆이나 아래의 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_spanning_its_table_s_margins", detail: CAPTION_SPANNING_ITS_TABLE_S_MARGINS, message: "표의 여백까지 가로지르는 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_without_its_width", detail: CAPTION_WITHOUT_ITS_WIDTH, message: "폭 정보가 없는 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_holding_a_table_or_object", detail: CAPTION_HOLDING_A_TABLE_OR_OBJECT, message: "표 또는 개체를 포함한 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_paragraph_without_its_line_positions", detail: CAPTION_PARAGRAPH_WITHOUT_ITS_LINE_POSITIONS, message: "줄 위치가 없는 캡션 문단을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
    Reason { code: "caption_whose_lines_restart", detail: CAPTION_WHOSE_LINES_RESTART, message: "줄 좌표가 다시 시작되는 캡션을 표시하지 못했습니다.", action: "원본의 캡션과 대조해 주세요. 필요하면 캡션을 표 위의 일반 문단으로 옮겨 다시 저장해 보세요." },
];
